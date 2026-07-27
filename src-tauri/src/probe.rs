use crate::media;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaTrack {
    pub stream_index: u32,
    pub kind: String,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub is_default: bool,
    pub is_forced: bool,
    pub is_hearing_impaired: bool,
    pub is_visual_impaired: bool,
    pub is_text: bool,
    /// Audio channel count; 0 for subtitle tracks. Used to pick an export
    /// bitrate — 192k across six channels is audibly poor.
    pub channels: u32,
}

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
    pub tracks: Vec<MediaTrack>,
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
    index: Option<u32>,
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    channels: Option<u32>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    tags: Option<HashMap<String, String>>,
    disposition: Option<FfDisposition>,
}

#[derive(Deserialize, Default)]
struct FfDisposition {
    #[serde(default)]
    default: u8,
    #[serde(default)]
    forced: u8,
    #[serde(default)]
    hearing_impaired: u8,
    #[serde(default)]
    visual_impaired: u8,
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
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            path,
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
    let tracks = parsed
        .streams
        .iter()
        .filter_map(|stream| {
            let kind = stream.codec_type.as_deref()?;
            if kind != "audio" && kind != "subtitle" {
                return None;
            }
            let codec = stream.codec_name.clone().unwrap_or_default();
            let tags = stream.tags.as_ref();
            let disposition = stream.disposition.as_ref();
            let text_subtitle = matches!(
                codec.as_str(),
                "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "mov_text" | "text"
            );
            Some(MediaTrack {
                stream_index: stream.index.unwrap_or(0),
                kind: kind.to_string(),
                codec,
                language: tags
                    .and_then(|t| t.get("language").or_else(|| t.get("LANGUAGE")))
                    .cloned(),
                title: tags
                    .and_then(|t| t.get("title").or_else(|| t.get("TITLE")))
                    .cloned(),
                is_default: disposition.map(|d| d.default != 0).unwrap_or(false),
                is_forced: disposition.map(|d| d.forced != 0).unwrap_or(false),
                is_hearing_impaired: disposition
                    .map(|d| d.hearing_impaired != 0)
                    .unwrap_or(false),
                is_visual_impaired: disposition.map(|d| d.visual_impaired != 0).unwrap_or(false),
                is_text: kind == "subtitle" && text_subtitle,
                channels: stream.channels.unwrap_or(0),
            })
        })
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
        tracks,
    })
}

pub fn preferred_audio_stream(info: &VideoInfo) -> Option<u32> {
    info.tracks
        .iter()
        .find(|track| track.kind == "audio" && track.is_default && !track.is_visual_impaired)
        .or_else(|| {
            info.tracks
                .iter()
                .find(|track| track.kind == "audio" && !track.is_visual_impaired)
        })
        .or_else(|| info.tracks.iter().find(|track| track.kind == "audio"))
        .map(|track| track.stream_index)
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
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "packet=pts_time,flags",
                "-of",
                "csv=p=0",
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
