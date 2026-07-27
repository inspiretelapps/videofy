use crate::content::{
    merge_events, stable_id, ContentCategory, ContentEvent, EventAction, Evidence,
};
use crate::media::ScanHost;
use crate::{media, probe};
use ort::{inputs, session::Session, value::Tensor};
use serde::Serialize;
use std::io::{Read, Write};

const MODEL_URL: &str = "https://huggingface.co/andrelgomes/yamnet-onnx/resolve/main/yamnet.onnx";
const LABELS_URL: &str =
    "https://raw.githubusercontent.com/tensorflow/models/master/research/audioset/yamnet/yamnet_class_map.csv";
/// Bumped when the risk table or chunking changes, so an old noisy cache is not
/// silently reused after recalibration.
const MODEL_VERSION: &str = "yamnet-onnx-risk-v2";
const SAMPLE_RATE: usize = 16_000;
const CHUNK_SECONDS: usize = 8;
/// YAMNet scores a 0.96 s patch every 0.48 s, so the final patch of a chunk
/// cannot start later than `CHUNK_SECONDS - 0.96`. Carrying that much audio
/// into the next chunk removes the recurring under-covered tail.
const OVERLAP_SECONDS: f64 = 0.96;
const FRAME_HOP: f64 = 0.48;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AudioEventResult {
    pub events: Vec<ContentEvent>,
    pub model: String,
    pub warnings: Vec<String>,
}

/// Per-label score distribution, collected only for `scan_report`. This is the
/// data you need to choose thresholds instead of guessing them: for each label,
/// how many 0.48 s frames scored above 0.1, 0.2, … 0.9.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LabelStat {
    pub label: String,
    pub buckets: [u32; 9],
    pub max_score: f32,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioScanStats {
    pub labels: Vec<LabelStat>,
    pub frames: u64,
}

struct RiskRule {
    /// Exact AudioSet display name. Substring matching is deliberately avoided:
    /// it was matching "Smash, crash" from "crash" and flooding every action
    /// scene with severity-3 violence.
    label: &'static str,
    category: ContentCategory,
    severity: u8,
    threshold: f32,
}

const fn rule(
    label: &'static str,
    category: ContentCategory,
    severity: u8,
    threshold: f32,
) -> RiskRule {
    RiskRule {
        label,
        category,
        severity,
        threshold,
    }
}

/// Deliberately narrow. Every label here is one YAMNet identifies reliably AND
/// that implies something about content rather than about foley.
///
/// Removed in the v2 recalibration, all of which fire constantly on ordinary
/// film sound design: Smash/crash, Thump/thud, Slap/smack, Breaking, Glass,
/// Shatter, Bang, Boom, Screech. A door closing is not violence.
///
/// Thresholds are principled starting points, not calibrated values. Run
/// `scan_report` over annotated movies and set them from the histogram.
const RISK_RULES: &[RiskRule] = &[
    // Distress vocalisations.
    rule("Screaming", ContentCategory::Frightening, 3, 0.40),
    rule("Wail, moan", ContentCategory::Frightening, 2, 0.50),
    rule("Crying, sobbing", ContentCategory::Frightening, 2, 0.45),
    rule("Whimper", ContentCategory::Frightening, 1, 0.50),
    rule(
        "Baby cry, infant cry",
        ContentCategory::Frightening,
        1,
        0.55,
    ),
    rule("Roar", ContentCategory::Frightening, 2, 0.50),
    rule("Growling", ContentCategory::Frightening, 2, 0.50),
    // Weapons — what YAMNet is genuinely good at.
    rule("Gunshot, gunfire", ContentCategory::Violence, 3, 0.40),
    rule("Machine gun", ContentCategory::Violence, 3, 0.40),
    rule("Fusillade", ContentCategory::Violence, 3, 0.45),
    rule("Artillery fire", ContentCategory::Violence, 3, 0.45),
    rule("Explosion", ContentCategory::Violence, 3, 0.45),
    // Highly ambiguous; kept only at a bar most films never reach.
    rule("Groan", ContentCategory::Sexual, 2, 0.65),
];

#[tauri::command]
pub async fn analyze_audio_events(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
) -> Result<AudioEventResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        analyze_with_host(&app, &path, duration, false).map(|(result, _)| result)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Shared entry point. `collect_stats` bypasses the cache and returns the
/// per-label score histogram used for threshold calibration.
pub fn analyze_with_host(
    host: &dyn ScanHost,
    path: &str,
    duration: f64,
    collect_stats: bool,
) -> Result<(AudioEventResult, Option<AudioScanStats>), String> {
    let info = probe::probe_sync(path)?;
    let audio_stream = probe::preferred_audio_stream(&info).ok_or("no audio track found")?;
    let cache =
        media::cache_dir_for(host, path)?.join(format!("{MODEL_VERSION}-{audio_stream}.json"));
    let _guard = media::JobGuard::acquire(format!("audio-events:{}", cache.display()))?;
    if !collect_stats {
        if let Ok(bytes) = std::fs::read(&cache) {
            if let Ok(events) = serde_json::from_slice::<Vec<ContentEvent>>(&bytes) {
                return Ok((
                    AudioEventResult {
                        events,
                        model: "YAMNet".into(),
                        warnings: Vec::new(),
                    },
                    None,
                ));
            }
        }
    }

    let models = host.models_dir()?;
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let model_path = models.join("yamnet.onnx");
    let labels_path = models.join("yamnet_class_map.csv");
    if !model_path.exists() {
        download(host, MODEL_URL, &model_path, "audio-model-download")?;
    }
    if !labels_path.exists() {
        download(host, LABELS_URL, &labels_path, "audio-labels-download")?;
    }
    let labels = load_labels(&labels_path)?;
    let mut session = Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(&model_path)
        .map_err(|e| format!("Could not load YAMNet: {e}"))?;

    let ffmpeg = media::ffmpeg_path();
    let map = format!("0:{audio_stream}");
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-v",
            "error",
            "-nostats",
            "-i",
            path,
            "-map",
            &map,
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "-",
        ],
    )?;
    let stderr = media::drain_stderr(&mut child);
    let mut reader = std::io::BufReader::with_capacity(
        SAMPLE_RATE * CHUNK_SECONDS * std::mem::size_of::<f32>(),
        child
            .stdout
            .take()
            .ok_or("ffmpeg produced no audio samples")?,
    );
    let mut raw = vec![0u8; SAMPLE_RATE * CHUNK_SECONDS * 4];
    let overlap_samples = (OVERLAP_SECONDS * SAMPLE_RATE as f64) as usize;
    let mut carry: Vec<f32> = Vec::new();
    let mut carry_start_sample = 0usize;
    let mut events = Vec::new();
    let mut stats: std::collections::HashMap<usize, LabelStat> = std::collections::HashMap::new();
    let mut frames_seen = 0u64;
    loop {
        let mut used = 0;
        while used < raw.len() {
            let count = reader.read(&mut raw[used..]).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            used += count;
        }
        used -= used % 4;
        if used == 0 {
            // Nothing new. The carry has already been scored as the tail of the
            // previous chunk, and feeding a lone 0.96 s buffer to the model
            // buys nothing while adding a short-input edge case.
            break;
        }
        let mut samples: Vec<f32> = Vec::with_capacity(carry.len() + used / 4);
        samples.extend_from_slice(&carry);
        samples.extend(
            raw[..used]
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        );
        let chunk_start = carry_start_sample as f64 / SAMPLE_RATE as f64;
        let chunk_len = samples.len();
        let input = Tensor::from_array(([chunk_len], samples.into_boxed_slice()))
            .map_err(|e| e.to_string())?;
        let outputs = session
            .run(inputs![input])
            .map_err(|e| format!("YAMNet inference failed: {e}"))?;
        let (shape, scores) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let classes = shape.get(1).copied().unwrap_or(labels.len() as i64) as usize;
        let frames = if classes > 0 {
            scores.len() / classes
        } else {
            0
        };
        for frame in 0..frames {
            frames_seen += 1;
            let frame_time = chunk_start + frame as f64 * FRAME_HOP;
            let row = &scores[frame * classes..(frame + 1) * classes];
            if collect_stats {
                for (label_index, score) in row.iter().copied().enumerate() {
                    if score < 0.10 {
                        continue;
                    }
                    let Some(label) = labels.get(label_index) else {
                        continue;
                    };
                    let entry = stats.entry(label_index).or_insert_with(|| LabelStat {
                        label: label.clone(),
                        buckets: [0; 9],
                        max_score: 0.0,
                    });
                    entry.max_score = entry.max_score.max(score);
                    for (bucket, slot) in entry.buckets.iter_mut().enumerate() {
                        if score >= (bucket as f32 + 1.0) / 10.0 {
                            *slot += 1;
                        }
                    }
                }
            }
            for (label_index, score) in row.iter().copied().enumerate() {
                let Some(label) = labels.get(label_index) else {
                    continue;
                };
                let Some(risk) = risk_rule(label) else {
                    continue;
                };
                if score < risk.threshold {
                    continue;
                }
                let start = (frame_time - 0.35).max(0.0);
                let end = (frame_time + 1.25).min(duration);
                // Confidence that the labelled sound occurred, not that the
                // scene is unsuitable. Kept below the text/guide sources on
                // purpose — a sound alone should never outrank a caption.
                let confidence = (0.30 + score as f64 * 0.55).min(0.90);
                let reason = format!("Sound detected: {label}");
                events.push(ContentEvent {
                    id: stable_id("yamnet", risk.category, start, end, label),
                    start,
                    end,
                    peak_time: frame_time,
                    category: risk.category,
                    severity: risk.severity,
                    confidence,
                    reason,
                    suggested_action: EventAction::Review,
                    evidence: vec![Evidence {
                        source: "YAMNet audio classifier".into(),
                        label: label.clone(),
                        detail: Some(format!("Model score {:.0}%", score * 100.0)),
                        confidence,
                    }],
                    source_key: format!("yamnet:{:?}", risk.category),
                });
            }
        }
        let pct = if duration > 0.0 {
            ((chunk_start + CHUNK_SECONDS as f64) / duration * 100.0).min(100.0)
        } else {
            0.0
        };
        host.emit(
            "audio-events-progress",
            serde_json::json!({ "t": chunk_start, "pct": pct }),
        );
        if used < raw.len() {
            break;
        }
        if chunk_len > overlap_samples {
            let keep_from = chunk_len - overlap_samples;
            // `samples` was moved into the tensor, so rebuild the carry from
            // the tail of the raw buffer we just read.
            let tail_floats = overlap_samples.min(used / 4);
            carry = raw[used - tail_floats * 4..used]
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                .collect();
            carry_start_sample += keep_from;
        } else {
            carry.clear();
            carry_start_sample += chunk_len;
        }
    }
    media::wait_checked(child, "semantic audio scan", stderr)?;
    let events = merge_events(events);
    if !collect_stats {
        let _ = std::fs::write(&cache, serde_json::to_vec(&events).unwrap_or_default());
    }
    let collected = collect_stats.then(|| {
        let mut labels: Vec<LabelStat> = stats.into_values().collect();
        labels.sort_by(|a, b| b.buckets[0].cmp(&a.buckets[0]));
        AudioScanStats {
            labels,
            frames: frames_seen,
        }
    });
    Ok((
        AudioEventResult {
            events,
            model: "YAMNet".into(),
            warnings: Vec::new(),
        },
        collected,
    ))
}

fn download(
    host: &dyn ScanHost,
    url: &str,
    destination: &std::path::Path,
    event_name: &str,
) -> Result<(), String> {
    let temp = destination.with_extension(format!("tmp-{}", std::process::id()));
    let mut response = reqwest::blocking::Client::builder()
        .user_agent("Videofy/0.2")
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .map_err(|e| format!("Model download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Model download failed: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let count = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        downloaded += count as u64;
        let pct = if total > 0 {
            downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        host.emit(
            event_name,
            serde_json::json!({ "downloaded": downloaded, "total": total, "pct": pct }),
        );
    }
    std::fs::rename(&temp, destination).map_err(|e| e.to_string())
}

fn load_labels(path: &std::path::Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let first_comma = line.find(',')?;
            let rest = &line[first_comma + 1..];
            let second_comma = rest.find(',')?;
            Some(
                rest[second_comma + 1..]
                    .trim()
                    .trim_matches('"')
                    .replace("\"\"", "\""),
            )
        })
        .collect())
}

fn risk_rule(label: &str) -> Option<&'static RiskRule> {
    RISK_RULES
        .iter()
        .find(|rule| rule.label.eq_ignore_ascii_case(label.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_risk_labels_without_treating_speech_as_unsafe() {
        assert_eq!(
            risk_rule("Gunshot, gunfire").map(|r| r.category),
            Some(ContentCategory::Violence)
        );
        assert!(risk_rule("Speech").is_none());
    }

    #[test]
    fn ordinary_foley_no_longer_counts_as_violence() {
        // Every one of these fired at 0.20 in v1 and flooded the review list.
        for label in [
            "Smash, crash",
            "Thump, thud",
            "Slap, smack",
            "Breaking",
            "Glass",
            "Shatter",
            "Bang",
            "Boom",
        ] {
            assert!(risk_rule(label).is_none(), "{label} should not be a risk");
        }
    }

    #[test]
    fn matches_whole_label_not_substring() {
        // "crash" used to match "Smash, crash"; a partial name must not match.
        assert!(risk_rule("crash").is_none());
        assert!(risk_rule("gun").is_none());
        assert!(risk_rule("Screaming").is_some());
    }

    #[test]
    fn every_threshold_is_above_the_v1_noise_floor() {
        for rule in RISK_RULES {
            assert!(
                rule.threshold >= 0.40,
                "{} threshold {} is back in noise territory",
                rule.label,
                rule.threshold
            );
        }
    }
}
