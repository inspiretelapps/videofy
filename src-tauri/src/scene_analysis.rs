use crate::content::{
    merge_events, stable_id, ContentCategory, ContentEvent, EventAction, Evidence,
};
use crate::media;
use crate::media::ScanHost;
use image::GenericImageView;
use serde::Serialize;
use std::io::BufRead;

const SCANNER_VERSION: &str = "scene-risk-v1";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SceneAnalysisResult {
    pub events: Vec<ContentEvent>,
    pub frames_scanned: usize,
    pub verifier: String,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct FrameRisk {
    time: f64,
    dark_fraction: f64,
    red_fraction: f64,
    skin_fraction: f64,
}

#[tauri::command]
pub async fn analyze_scenes(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
) -> Result<SceneAnalysisResult, String> {
    tauri::async_runtime::spawn_blocking(move || analyze_with_host(&app, &path, duration))
        .await
        .map_err(|e| e.to_string())?
}

pub fn analyze_with_host(
    host: &dyn ScanHost,
    path: &str,
    duration: f64,
) -> Result<SceneAnalysisResult, String> {
    let cache = media::cache_dir_for(host, path)?.join(format!("{SCANNER_VERSION}.json"));
    let _guard = media::JobGuard::acquire(format!("scene-analysis:{}", cache.display()))?;
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(events) = serde_json::from_slice::<Vec<ContentEvent>>(&bytes) {
            return Ok(SceneAnalysisResult {
                events,
                frames_scanned: 0,
                verifier: "built-in visual risk pass (cached)".into(),
                warnings: vec![
                    "Visual results are conservative, low-confidence review clues.".into(),
                ],
            });
        }
    }

    let cache_dir = media::cache_dir_for(host, path)?;
    let frames_dir = cache_dir.join(format!("scene-frames-{}", std::process::id()));
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    let pattern = frames_dir.join("frame-%06d.jpg");
    let pattern_string = pattern.to_string_lossy().to_string();
    let ffmpeg = media::ffmpeg_path();
    let filter = "select='eq(n,0)+gt(scene,0.28)',scale=320:-2,showinfo";
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-v",
            "info",
            "-nostats",
            "-i",
            path,
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-vf",
            filter,
            "-fps_mode",
            "vfr",
            "-q:v",
            "5",
            &pattern_string,
        ],
    )?;
    let stderr = child
        .stderr
        .take()
        .ok_or("ffmpeg produced no scene metadata")?;
    let mut frame_times = Vec::new();
    let mut error_tail = std::collections::VecDeque::new();
    for line in std::io::BufReader::new(stderr)
        .lines()
        .map_while(Result::ok)
    {
        if line.contains("showinfo") {
            if let Some(time) = parse_showinfo_time(&line) {
                frame_times.push(time);
            }
        }
        if error_tail.len() >= 8 {
            error_tail.pop_front();
        }
        error_tail.push_back(line);
    }
    media::wait_checked(child, "scene-frame extraction", None).map_err(|error| {
        format!(
            "{error}\n{}",
            error_tail.into_iter().collect::<Vec<_>>().join("\n")
        )
    })?;

    let mut paths: Vec<_> = std::fs::read_dir(&frames_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|v| v.to_str()) == Some("jpg"))
        .collect();
    paths.sort();
    let mut warnings = vec!["Visual results are conservative, low-confidence review clues.".into()];

    // Frames are paired with showinfo timestamps by position. If ffmpeg wrote a
    // different number of frames than it logged, that pairing is meaningless —
    // and the previous behaviour (evenly spacing the missing ones across the
    // runtime) invented timestamps that looked plausible and were wrong. The
    // last timestamp bug in this function went unnoticed through six modules,
    // so drop the unpaired frames and say so rather than guess.
    if frame_times.len() != paths.len() {
        warnings.push(format!(
            "Scene timestamps incomplete: ffmpeg wrote {} frames but logged {} timestamps; \
             {} frame(s) were skipped rather than given estimated times.",
            paths.len(),
            frame_times.len(),
            paths.len().saturating_sub(frame_times.len()),
        ));
    }
    let usable = paths.len().min(frame_times.len());
    let frames_scanned = usable;
    let mut risks = Vec::new();
    for (index, frame_path) in paths.iter().take(usable).enumerate() {
        let time = frame_times[index];
        if let Ok(risk) = inspect_frame(frame_path, time) {
            risks.push(risk);
        }
        if index % 20 == 0 || index + 1 == frames_scanned {
            let pct = if frames_scanned > 0 {
                (index + 1) as f64 / frames_scanned as f64 * 100.0
            } else {
                100.0
            };
            host.emit(
                "scene-analysis-progress",
                serde_json::json!({ "pct": pct, "frames": index + 1 }),
            );
        }
    }

    let events = risks
        .iter()
        .filter_map(|risk| heuristic_event(risk, duration))
        .collect();
    let _ = std::fs::remove_dir_all(&frames_dir);
    let events = merge_events(events);
    let _ = std::fs::write(&cache, serde_json::to_vec(&events).unwrap_or_default());
    Ok(SceneAnalysisResult {
        events,
        frames_scanned,
        verifier: "built-in visual risk pass".into(),
        warnings,
    })
}

fn inspect_frame(path: &std::path::Path, time: f64) -> Result<FrameRisk, String> {
    let image = image::open(path).map_err(|e| e.to_string())?;
    let mut dark = 0usize;
    let mut red = 0usize;
    let mut skin = 0usize;
    let mut total = 0usize;
    for (_, _, pixel) in image.pixels().step_by(16) {
        let [r, g, b, _] = pixel.0;
        let r = r as f64;
        let g = g as f64;
        let b = b as f64;
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if luminance < 42.0 {
            dark += 1;
        }
        if r > 75.0 && r > g * 1.38 && r > b * 1.3 && r - g > 24.0 {
            red += 1;
        }
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if r > 70.0
            && g > 35.0
            && b > 20.0
            && r > g
            && g > b
            && max - min > 15.0
            && (r - g).abs() > 10.0
        {
            skin += 1;
        }
        total += 1;
    }
    Ok(FrameRisk {
        time,
        dark_fraction: dark as f64 / total.max(1) as f64,
        red_fraction: red as f64 / total.max(1) as f64,
        skin_fraction: skin as f64 / total.max(1) as f64,
    })
}

fn parse_showinfo_time(line: &str) -> Option<f64> {
    let marker = "pts_time:";
    let start = line.find(marker)? + marker.len();
    let token = line[start..].split_whitespace().next()?;
    token.parse().ok()
}

fn heuristic_event(risk: &FrameRisk, duration: f64) -> Option<ContentEvent> {
    let (category, severity, confidence, reason) = if risk.red_fraction >= 0.095 {
        (
            ContentCategory::Violence,
            2,
            0.48,
            "Possible blood, fire, or strongly red violent imagery",
        )
    } else if risk.skin_fraction >= 0.43 {
        (
            ContentCategory::Nudity,
            2,
            0.42,
            "Possible extensive visible skin or nudity",
        )
    } else if risk.dark_fraction >= 0.86 && risk.red_fraction >= 0.025 {
        (
            ContentCategory::Frightening,
            1,
            0.36,
            "Dark scene with potentially frightening visual contrast",
        )
    } else {
        return None;
    };
    let start = (risk.time - 2.0).max(0.0);
    let end = (risk.time + 4.0).min(duration);
    Some(ContentEvent {
        id: stable_id("visual-heuristic", category, start, end, reason),
        start,
        end,
        peak_time: risk.time,
        category,
        severity,
        confidence,
        reason: reason.into(),
        suggested_action: EventAction::Review,
        evidence: vec![Evidence {
            source: "scene visual scan".into(),
            label: format!(
                "red {:.0}% · skin-like {:.0}% · dark {:.0}%",
                risk.red_fraction * 100.0,
                risk.skin_fraction * 100.0,
                risk.dark_fraction * 100.0
            ),
            detail: Some("Low-confidence visual clue; inspect the frames.".into()),
            confidence,
        }],
        source_key: format!("visual:{category:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_showinfo_timestamp() {
        let line =
            "[Parsed_showinfo_2 @ 0x123] n: 1 pts: 24576 pts_time:2 duration:512 duration_time:0.04";
        assert_eq!(parse_showinfo_time(line), Some(2.0));
    }
}
