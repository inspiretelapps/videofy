use crate::media;
use serde::{Deserialize, Serialize};
use std::io::Read;
use tauri::Emitter;

const SAMPLE_RATE: u32 = 8000;
pub const BUCKET_DT: f64 = 0.025;
const SAMPLES_PER_BUCKET: u32 = (SAMPLE_RATE as f64 * BUCKET_DT) as u32; // 200

/// Per-channel peak amplitude (0-255) every 25ms, for the stereo timeline
/// waveform. 8kHz decode is plenty for a visual peak display and keeps a
/// two-hour movie at ~240k buckets per channel.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub dt: f64,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

#[tauri::command]
pub async fn get_waveform(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
) -> Result<Waveform, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache = media::cache_dir_for(&app, &path)?.join("waveform.json");
        let _guard = media::JobGuard::acquire(format!("waveform:{}", cache.display()))?;
        if let Ok(bytes) = std::fs::read(&cache) {
            if let Ok(wf) = serde_json::from_slice::<Waveform>(&bytes) {
                if !wf.left.is_empty() {
                    return Ok(wf);
                }
            }
        }

        let ffmpeg = media::ffmpeg_path();
        let rate = SAMPLE_RATE.to_string();
        let mut child = media::spawn(
            &ffmpeg,
            &[
                "-v", "error", "-nostats",
                "-i", &path,
                "-map", "0:a:0",
                "-vn", "-sn", "-dn",
                "-ac", "2",
                "-ar", &rate,
                "-c:a", "pcm_s16le",
                "-f", "s16le",
                "-",
            ],
        )?;
        let stderr_drain = media::drain_stderr(&mut child);
        let stdout = child.stdout.take().ok_or("no stdout from ffmpeg")?;

        let mut wf = Waveform {
            dt: BUCKET_DT,
            left: Vec::with_capacity((duration / BUCKET_DT) as usize + 16),
            right: Vec::with_capacity((duration / BUCKET_DT) as usize + 16),
        };
        let mut reader = std::io::BufReader::with_capacity(1 << 16, stdout);
        let mut buf = [0u8; 1 << 16];
        let mut carry: Vec<u8> = Vec::new();
        let mut peak_l: i32 = 0;
        let mut peak_r: i32 = 0;
        let mut frames_in_bucket: u32 = 0;
        let mut total_frames: u64 = 0;
        let mut frames_since_emit: u64 = 0;

        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            carry.extend_from_slice(&buf[..n]);
            let usable = carry.len() - (carry.len() % 4);
            for frame in carry[..usable].chunks_exact(4) {
                let l = i16::from_le_bytes([frame[0], frame[1]]) as i32;
                let r = i16::from_le_bytes([frame[2], frame[3]]) as i32;
                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
                frames_in_bucket += 1;
                if frames_in_bucket == SAMPLES_PER_BUCKET {
                    wf.left.push((peak_l * 255 / 32767).min(255) as u8);
                    wf.right.push((peak_r * 255 / 32767).min(255) as u8);
                    peak_l = 0;
                    peak_r = 0;
                    frames_in_bucket = 0;
                }
            }
            total_frames += (usable / 4) as u64;
            frames_since_emit += (usable / 4) as u64;
            carry.drain(..usable);
            // one progress event per ~10s of audio
            if frames_since_emit >= SAMPLE_RATE as u64 * 10 {
                frames_since_emit = 0;
                let t = total_frames as f64 / SAMPLE_RATE as f64;
                let pct = if duration > 0.0 { (t / duration * 100.0).min(100.0) } else { 0.0 };
                let _ = app.emit("waveform-progress", serde_json::json!({ "t": t, "pct": pct }));
            }
        }
        if frames_in_bucket > 0 {
            wf.left.push((peak_l * 255 / 32767).min(255) as u8);
            wf.right.push((peak_r * 255 / 32767).min(255) as u8);
        }
        media::wait_checked(child, "waveform extraction", stderr_drain)?;
        if wf.left.is_empty() {
            return Err("waveform extraction produced no audio data".into());
        }
        let _ = std::fs::write(&cache, serde_json::to_vec(&wf).unwrap_or_default());
        let _ = app.emit("waveform-progress", serde_json::json!({ "t": duration, "pct": 100.0 }));
        Ok(wf)
    })
    .await
    .map_err(|e| e.to_string())?
}
