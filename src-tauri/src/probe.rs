use crate::media;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub path: String,
    pub file_name: String,
    pub container: String,
    pub duration: f64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_tracks: u32,
}

#[derive(Deserialize)]
struct FfprobeOut {
    format: FfFormat,
    streams: Vec<FfStream>,
}

#[derive(Deserialize)]
struct FfFormat {
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
}

#[derive(Deserialize)]
struct FfStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
}

fn parse_rate(rate: &Option<String>) -> Option<f64> {
    let r = rate.as_deref()?;
    let mut parts = r.splitn(2, '/');
    let num: f64 = parts.next()?.parse().ok()?;
    let den: f64 = parts.next().unwrap_or("1").parse().ok()?;
    if den == 0.0 || num == 0.0 {
        None
    } else {
        Some(num / den)
    }
}

#[tauri::command]
pub async fn probe_video(path: String) -> Result<VideoInfo, String> {
    tauri::async_runtime::spawn_blocking(move || probe_sync(&path))
        .await
        .map_err(|e| e.to_string())?
}

pub fn probe_sync(path: &str) -> Result<VideoInfo, String> {
    let out = std::process::Command::new(media::ffprobe_path())
        .args([
            "-v", "error", "-print_format", "json", "-show_format", "-show_streams", path,
        ])
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let parsed: FfprobeOut =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("bad ffprobe output: {e}"))?;

    let video = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or("no video stream found in this file")?;
    let audios: Vec<&FfStream> = parsed
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();

    let duration = parsed
        .format
        .duration
        .as_deref()
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| video.duration.as_deref().and_then(|d| d.parse().ok()))
        .ok_or("could not determine duration")?;

    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    Ok(VideoInfo {
        path: path.to_string(),
        file_name,
        container: parsed.format.format_name.unwrap_or_default(),
        duration,
        size_bytes: parsed
            .format
            .size
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        fps: parse_rate(&video.avg_frame_rate)
            .or_else(|| parse_rate(&video.r_frame_rate))
            .unwrap_or(24.0),
        video_codec: video.codec_name.clone().unwrap_or_default(),
        audio_codec: audios
            .first()
            .and_then(|a| a.codec_name.clone())
            .unwrap_or_default(),
        audio_tracks: audios.len() as u32,
    })
}

/// All video keyframe timestamps, used to snap lossless cut points.
#[tauri::command]
pub async fn get_keyframes(app: tauri::AppHandle, path: String) -> Result<Vec<f64>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache = media::cache_dir_for(&app, &path)?.join("keyframes.json");
        if let Ok(bytes) = std::fs::read(&cache) {
            if let Ok(kf) = serde_json::from_slice::<Vec<f64>>(&bytes) {
                return Ok(kf);
            }
        }
        let out = std::process::Command::new(media::ffprobe_path())
            .args([
                "-v", "error",
                "-select_streams", "v:0",
                "-show_entries", "packet=pts_time,flags",
                "-of", "csv=p=0",
                &path,
            ])
            .output()
            .map_err(|e| format!("failed to run ffprobe: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "keyframe scan failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut kf: Vec<f64> = text
            .lines()
            .filter_map(|line| {
                let mut cols = line.split(',');
                let pts = cols.next()?.trim();
                let flags = cols.next()?.trim();
                if flags.contains('K') {
                    pts.parse::<f64>().ok()
                } else {
                    None
                }
            })
            .collect();
        kf.sort_by(|a, b| a.partial_cmp(b).unwrap());
        kf.dedup();
        let _ = std::fs::write(&cache, serde_json::to_vec(&kf).unwrap_or_default());
        Ok(kf)
    })
    .await
    .map_err(|e| e.to_string())?
}
