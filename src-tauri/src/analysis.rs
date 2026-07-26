use crate::media;
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use tauri::Emitter;

/// Momentary (400ms) and short-term (3s) EBU R128 loudness sampled every 100ms.
#[derive(Serialize, Deserialize, Default)]
struct Series {
    t: Vec<f64>,
    m: Vec<f64>,
    s: Vec<f64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScareCandidate {
    pub id: u32,
    pub start: f64,
    pub end: f64,
    pub peak_time: f64,
    /// 1-100, how confident we are this is a jump-scare-like event
    pub score: u32,
    /// how far the momentary loudness jumped above the preceding baseline, in LU
    pub jump_lu: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub duration: f64,
    pub envelope_dt: f64,
    /// max momentary loudness (LUFS) per bucket, for the timeline waveform
    pub envelope: Vec<f64>,
    pub candidates: Vec<ScareCandidate>,
}

#[tauri::command]
pub async fn analyze_audio(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    sensitivity: Option<f64>,
) -> Result<AnalysisResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let series = ensure_series(&app, &path, duration)?;
        let sens = sensitivity.unwrap_or(0.5).clamp(0.0, 1.0);
        let candidates = detect(&series, sens);
        Ok(AnalysisResult {
            duration,
            envelope_dt: envelope_dt(duration),
            envelope: envelope(&series, duration),
            candidates,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Runs ffmpeg's ebur128 filter over the first audio track (cached per file).
fn ensure_series(app: &tauri::AppHandle, path: &str, duration: f64) -> Result<Series, String> {
    let cache = media::cache_dir_for(app, path)?.join("loudness.json");
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(series) = serde_json::from_slice::<Series>(&bytes) {
            if !series.t.is_empty() {
                return Ok(series);
            }
        }
    }

    let ffmpeg = media::ffmpeg_path();
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-hide_banner", "-nostats",
            "-i", path,
            "-map", "0:a:0",
            "-vn", "-sn", "-dn",
            "-af", "ebur128",
            "-f", "null", "-",
        ],
    )?;

    let stderr = child.stderr.take().ok_or("no stderr from ffmpeg")?;
    let reader = std::io::BufReader::new(stderr);
    let mut series = Series::default();
    let mut lines_since_emit = 0u32;
    let mut error_tail: Vec<String> = Vec::new();

    for line in reader.lines().map_while(Result::ok) {
        if let Some((t, m, s)) = parse_ebur128_line(&line) {
            series.t.push(t);
            series.m.push(m);
            series.s.push(s);
            lines_since_emit += 1;
            if lines_since_emit >= 100 {
                lines_since_emit = 0;
                let pct = if duration > 0.0 { (t / duration * 100.0).min(100.0) } else { 0.0 };
                let _ = app.emit("analysis-progress", serde_json::json!({ "t": t, "pct": pct }));
            }
        } else {
            error_tail.push(line);
            if error_tail.len() > 8 {
                error_tail.remove(0);
            }
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() || series.t.is_empty() {
        return Err(format!(
            "audio analysis failed:\n{}",
            error_tail.join("\n")
        ));
    }

    let _ = std::fs::write(&cache, serde_json::to_vec(&series).unwrap_or_default());
    let _ = app.emit("analysis-progress", serde_json::json!({ "t": duration, "pct": 100.0 }));
    Ok(series)
}

/// ebur128 lines look like:
/// `[Parsed_ebur128_0 @ 0x...] t: 2.10233  TARGET:-23 LUFS  M: -18.5 S: -19.2  I: -20.1 LUFS ...`
fn parse_ebur128_line(line: &str) -> Option<(f64, f64, f64)> {
    if !line.contains("t:") || !line.contains("M:") {
        return None;
    }
    let mut t = None;
    let mut m = None;
    let mut s = None;
    let mut tokens = line.split_whitespace().peekable();
    while let Some(tok) = tokens.next() {
        match tok {
            "t:" => t = tokens.peek().and_then(|v| v.parse::<f64>().ok()),
            "M:" => m = tokens.peek().and_then(|v| v.parse::<f64>().ok()),
            "S:" => s = tokens.peek().and_then(|v| v.parse::<f64>().ok()),
            _ => {}
        }
    }
    match (t, m, s) {
        (Some(t), Some(m), Some(s)) => Some((t, m, s)),
        _ => None,
    }
}

const SAMPLE_DT: f64 = 0.1;
const BASELINE_WINDOW_S: f64 = 20.0;
const BASELINE_GAP_S: f64 = 1.0;
const SPIKE_MERGE_GAP_S: f64 = 1.5;
const PRE_PAD_S: f64 = 3.0;
const POST_PAD_S: f64 = 1.5;
const CANDIDATE_MERGE_GAP_S: f64 = 3.0;

/// A jump scare reads as: sustained baseline loudness, then the momentary
/// level suddenly jumps far above it. We compare each 100ms momentary sample
/// against the median short-term loudness of the preceding ~20 seconds.
fn detect(series: &Series, sensitivity: f64) -> Vec<ScareCandidate> {
    let jump_threshold = 16.0 - 8.0 * sensitivity; // LU above baseline
    let min_loud = -24.0 - 8.0 * sensitivity; // absolute LUFS floor

    let n = series.t.len();
    let window = (BASELINE_WINDOW_S / SAMPLE_DT) as usize;
    let gap = (BASELINE_GAP_S / SAMPLE_DT) as usize;

    struct Spike {
        t: f64,
        jump: f64,
    }
    let mut spikes: Vec<Spike> = Vec::new();
    let mut sorted = Vec::with_capacity(window);

    for i in 0..n {
        if i < gap + window / 4 {
            continue; // not enough history for a baseline yet
        }
        let lo = i.saturating_sub(gap + window);
        let hi = i - gap;
        sorted.clear();
        sorted.extend(
            series.s[lo..hi]
                .iter()
                .copied()
                .filter(|v| v.is_finite() && *v > -90.0),
        );
        if sorted.len() < window / 4 {
            continue;
        }
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let baseline = sorted[sorted.len() / 2];
        let m = series.m[i];
        if !m.is_finite() {
            continue;
        }
        let jump = m - baseline;
        if jump >= jump_threshold && m >= min_loud {
            spikes.push(Spike { t: series.t[i], jump });
        }
    }

    // group spikes separated by small gaps into events
    let mut events: Vec<(f64, f64, f64)> = Vec::new(); // (start, end, peak_jump)
    for sp in &spikes {
        match events.last_mut() {
            Some((_, end, peak)) if sp.t - *end <= SPIKE_MERGE_GAP_S => {
                *end = sp.t;
                *peak = peak.max(sp.jump);
            }
            _ => events.push((sp.t, sp.t, sp.jump)),
        }
    }

    let total = series.t.last().copied().unwrap_or(0.0);
    let mut candidates: Vec<ScareCandidate> = Vec::new();
    for (start, end, peak_jump) in events {
        let c_start = (start - PRE_PAD_S).max(0.0);
        let c_end = (end + POST_PAD_S).min(total);
        let peak_time = spikes
            .iter()
            .filter(|s| s.t >= start && s.t <= end)
            .max_by(|a, b| a.jump.partial_cmp(&b.jump).unwrap())
            .map(|s| s.t)
            .unwrap_or(start);
        let score = ((peak_jump - jump_threshold) * 6.0 + 20.0).clamp(1.0, 100.0) as u32;
        match candidates.last_mut() {
            Some(prev) if c_start - prev.end <= CANDIDATE_MERGE_GAP_S => {
                prev.end = prev.end.max(c_end);
                prev.score = prev.score.max(score);
                prev.jump_lu = prev.jump_lu.max(peak_jump);
            }
            _ => candidates.push(ScareCandidate {
                id: candidates.len() as u32,
                start: c_start,
                end: c_end,
                peak_time,
                score,
                jump_lu: peak_jump,
            }),
        }
    }
    // re-number after merging
    for (i, c) in candidates.iter_mut().enumerate() {
        c.id = i as u32;
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_series(seconds: f64) -> Series {
        let n = (seconds / SAMPLE_DT) as usize;
        Series {
            t: (0..n).map(|i| i as f64 * SAMPLE_DT).collect(),
            m: vec![-50.0; n],
            s: vec![-50.0; n],
        }
    }

    #[test]
    fn detects_burst_out_of_silence() {
        let mut series = quiet_series(60.0);
        // loud burst at t = 30.0..31.5 over a -50 LUFS baseline
        for i in 300..315 {
            series.m[i] = -15.0;
            series.s[i] = -25.0;
        }
        let candidates = detect(&series, 0.5);
        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert!(c.start < 30.0 && c.end > 31.4, "padded range covers the burst");
        assert!(c.peak_time >= 30.0 && c.peak_time <= 31.5);
        assert!(c.score > 50, "a 35 LU jump should score high, got {}", c.score);
    }

    #[test]
    fn quiet_movie_has_no_candidates() {
        let series = quiet_series(60.0);
        assert!(detect(&series, 0.5).is_empty());
    }

    #[test]
    fn steady_loud_action_is_not_a_scare() {
        let n = 600;
        // constant -14 LUFS: loud, but the baseline is loud too — no jump
        let series = Series {
            t: (0..n).map(|i| i as f64 * SAMPLE_DT).collect(),
            m: vec![-14.0; n],
            s: vec![-14.0; n],
        };
        assert!(detect(&series, 0.5).is_empty());
    }
}

fn envelope_dt(duration: f64) -> f64 {
    (duration / 4000.0).max(SAMPLE_DT)
}

fn envelope(series: &Series, duration: f64) -> Vec<f64> {
    let dt = envelope_dt(duration);
    let n = (duration / dt).ceil() as usize + 1;
    let mut out = vec![-70.0f64; n];
    for (i, &t) in series.t.iter().enumerate() {
        let bucket = ((t / dt) as usize).min(n - 1);
        let m = series.m[i];
        if m.is_finite() && m > out[bucket] {
            out[bucket] = m;
        }
    }
    out
}
