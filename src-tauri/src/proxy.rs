use crate::probe::VideoInfo;
use crate::{media, probe};
use std::path::{Path, PathBuf};
use tauri::Emitter;

const PREVIEW_VERSION: &str = "preview-v5";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewStrategy {
    /// The source is already a clean, WebKit-compatible MP4-family file. A
    /// hard link inside the app cache exposes it through the tightly scoped
    /// asset protocol without copying or encoding the movie.
    DirectLink,
    /// 8-bit H.264 can be copied into a clean MP4. Only incompatible audio is
    /// encoded, which is normally many times faster than decoding every frame.
    Remux,
    /// Last resort for codecs WKWebView cannot play. HEVC belongs here:
    /// VideoToolbox can decode it, but the HTML video element still throws
    /// NotSupportedError even for a well-formed hvc1 Main 8-bit MP4.
    Transcode,
}

fn is_mp4_family(container: &str) -> bool {
    container
        .split(',')
        .any(|name| matches!(name.trim(), "mov" | "mp4" | "m4v"))
}

fn video_can_copy(info: &VideoInfo) -> bool {
    info.video_codec == "h264" && matches!(info.video_pixel_format.as_str(), "yuv420p" | "yuvj420p")
}

fn preferred_audio_codec(info: &VideoInfo) -> Option<&str> {
    let index = probe::preferred_audio_stream(info)?;
    info.tracks
        .iter()
        .find(|track| track.stream_index == index)
        .map(|track| track.codec.as_str())
}

fn choose_preview_strategy(info: &VideoInfo) -> PreviewStrategy {
    let clean_direct_source = is_mp4_family(&info.container)
        && video_can_copy(info)
        && info.video_tracks == 1
        && info.audio_tracks <= 1
        && !info.has_unsupported_preview_streams
        && info.chapter_count == 0
        && preferred_audio_codec(info)
            .map(|codec| codec == "aac")
            .unwrap_or(info.audio_tracks == 0);
    if clean_direct_source {
        PreviewStrategy::DirectLink
    } else if video_can_copy(info) {
        PreviewStrategy::Remux
    } else {
        PreviewStrategy::Transcode
    }
}

fn valid_cache_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() > 1024)
        .unwrap_or(false)
}

fn preview_is_webkit_playable(path: &Path) -> bool {
    probe::probe_sync(&path.to_string_lossy())
        .map(|info| video_can_copy(&info))
        .unwrap_or(false)
}

fn cached_playable_preview(dir: &Path, force_transcode: bool) -> Option<PathBuf> {
    let candidates = [
        dir.join(format!("{PREVIEW_VERSION}.mp4")),
        dir.join("preview-v4.mp4"),
        dir.join("proxy-v3.mp4"),
    ];
    let mut playable = None;
    for path in candidates {
        if !valid_cache_file(&path) {
            continue;
        }
        if preview_is_webkit_playable(&path) {
            if playable.is_none() {
                playable = Some(path);
            }
            continue;
        }
        // v4 remuxed HEVC into MP4. WKWebView rejects those files, so drop
        // the cache entry rather than serving it again.
        let _ = std::fs::remove_file(&path);
    }
    if force_transcode {
        None
    } else {
        playable
    }
}

/// Produces a WebKit-compatible viewing copy. This file is disposable and is
/// never used by export; cuts are always applied to the untouched source.
#[tauri::command]
pub async fn generate_proxy(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    source_height: u32,
    force_transcode: Option<bool>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let force_transcode = force_transcode.unwrap_or(false);
        let dir = media::cache_dir_for(&app, &path)?;
        let _guard = media::JobGuard::acquire(format!("proxy:{}", dir.display()))?;
        if let Some(cached) = cached_playable_preview(&dir, force_transcode) {
            return crate::preview_http::url_for(&app, &cached);
        }
        let preview = dir.join(format!("{PREVIEW_VERSION}.mp4"));
        if force_transcode {
            let _ = std::fs::remove_file(&preview);
        }

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{PREVIEW_VERSION}.tmp"))
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let tmp = dir.join(format!("{PREVIEW_VERSION}.tmp.{}.mp4", std::process::id()));
        let info = probe::probe_sync(&path)?;
        let strategy = if force_transcode {
            PreviewStrategy::Transcode
        } else {
            choose_preview_strategy(&info)
        };

        if strategy == PreviewStrategy::DirectLink {
            match std::fs::hard_link(&path, &preview) {
                Ok(()) => {
                    let _ = app.emit(
                        "proxy-progress",
                        serde_json::json!({ "t": duration, "pct": 100.0, "mode": "direct" }),
                    );
                    return crate::preview_http::url_for(&app, &preview);
                }
                // External volumes cannot be hard-linked into the local cache.
                // A clean stream-copy remux is still much faster than encoding.
                Err(_) => {}
            }
        }

        encode_preview(&app, &path, duration, source_height, &info, &tmp, strategy)?;

        std::fs::rename(&tmp, &preview).map_err(|e| e.to_string())?;
        let _ = app.emit(
            "proxy-progress",
            serde_json::json!({ "t": duration, "pct": 100.0 }),
        );
        crate::preview_http::url_for(&app, &preview)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn encode_preview(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    source_height: u32,
    info: &VideoInfo,
    tmp: &Path,
    strategy: PreviewStrategy,
) -> Result<(), String> {
    let mut remux_error = None;
    if strategy != PreviewStrategy::Transcode {
        match run_ffmpeg_preview(
            app,
            path,
            duration,
            source_height,
            info,
            tmp,
            PreviewStrategy::Remux,
            false,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                remux_error = Some(error);
                let _ = std::fs::remove_file(tmp);
            }
        }
    }
    match run_ffmpeg_preview(
        app,
        path,
        duration,
        source_height,
        info,
        tmp,
        PreviewStrategy::Transcode,
        false,
    ) {
        Ok(()) => return Ok(()),
        Err(vt_error) => {
            let _ = std::fs::remove_file(tmp);
            run_ffmpeg_preview(
                app,
                path,
                duration,
                source_height,
                info,
                tmp,
                PreviewStrategy::Transcode,
                true,
            )
            .map_err(|sw_error| match remux_error {
                Some(remux) => format!(
                    "Fast preview remux failed ({remux}); \
                     hardware preview encoding failed ({vt_error}); \
                     software preview encoding failed: {sw_error}"
                ),
                None => format!(
                    "Hardware preview encoding failed ({vt_error}); \
                     software preview encoding failed: {sw_error}"
                ),
            })
        }
    }
}

fn run_ffmpeg_preview(
    app: &tauri::AppHandle,
    path: &str,
    duration: f64,
    source_height: u32,
    info: &VideoInfo,
    tmp: &Path,
    strategy: PreviewStrategy,
    software: bool,
) -> Result<(), String> {
    let audio_map = probe::preferred_audio_stream(info)
        .map(|index| format!("0:{index}?"))
        .unwrap_or_else(|| "0:a:0?".into());
    let audio_can_copy = preferred_audio_codec(info) == Some("aac");
    let tmp_text = tmp.to_string_lossy().to_string();

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
    ];
    if strategy == PreviewStrategy::Transcode && !software {
        args.extend(["-hwaccel".into(), "videotoolbox".into()]);
    }
    args.extend([
        "-i".into(),
        path.into(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        audio_map,
    ]);

    if strategy == PreviewStrategy::Remux {
        args.extend(["-c:v".into(), "copy".into()]);
    } else if software {
        let target_height = source_height.min(360);
        let target_height = target_height - (target_height % 2);
        args.extend([
            "-vf".into(),
            format!("scale=-2:{}", target_height.max(2)),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-b:v".into(),
            "1000k".into(),
        ]);
    } else {
        let target_height = source_height.min(360);
        let target_height = target_height - (target_height % 2);
        args.extend([
            "-vf".into(),
            format!("scale=-2:{}", target_height.max(2)),
            "-c:v".into(),
            "h264_videotoolbox".into(),
            "-b:v".into(),
            "1000k".into(),
            "-allow_sw".into(),
            "1".into(),
        ]);
    }

    if strategy == PreviewStrategy::Remux && audio_can_copy {
        args.extend(["-c:a".into(), "copy".into()]);
    } else {
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-ac".into(),
            "2".into(),
        ]);
    }
    // Keep this output strictly audiovisual. A copied chapter `text` track
    // previously made WebKit play the picture without sound.
    args.extend([
        "-map_chapters".into(),
        "-1".into(),
        "-dn".into(),
        "-sn".into(),
        "-movflags".into(),
        "+faststart".into(),
        tmp_text,
    ]);

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let ffmpeg = media::ffmpeg_path();
    let mut child = media::spawn(&ffmpeg, &arg_refs)?;
    let stderr_drain = media::drain_stderr(&mut child);
    if let Some(stdout) = child.stdout.take() {
        let reader = std::io::BufReader::new(stdout);
        media::read_progress(reader, |t| {
            let pct = if duration > 0.0 {
                (t / duration * 100.0).min(100.0)
            } else {
                0.0
            };
            let mode = if strategy == PreviewStrategy::Remux {
                "remux"
            } else {
                "transcode"
            };
            let _ = app.emit(
                "proxy-progress",
                serde_json::json!({ "t": t, "pct": pct, "mode": mode }),
            );
        });
    }
    media::wait_checked(child, "preview generation", stderr_drain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::MediaTrack;

    fn info(container: &str, video: &str, audio: &str) -> VideoInfo {
        VideoInfo {
            path: String::new(),
            file_name: String::new(),
            container: container.into(),
            duration: 60.0,
            size_bytes: 0,
            width: 1920,
            height: 1080,
            fps: 24.0,
            video_codec: video.into(),
            video_pixel_format: "yuv420p".into(),
            audio_codec: audio.into(),
            video_tracks: 1,
            audio_tracks: 1,
            chapter_count: 0,
            has_unsupported_preview_streams: false,
            tracks: vec![MediaTrack {
                stream_index: 1,
                kind: "audio".into(),
                codec: audio.into(),
                language: Some("eng".into()),
                title: None,
                is_default: true,
                is_forced: false,
                is_hearing_impaired: false,
                is_visual_impaired: false,
                is_text: false,
                channels: 2,
            }],
        }
    }

    #[test]
    fn direct_links_only_clean_compatible_mp4() {
        assert_eq!(
            choose_preview_strategy(&info("mov,mp4,m4a,3gp,3g2,mj2", "h264", "aac")),
            PreviewStrategy::DirectLink
        );
        let mut with_subtitles = info("mov,mp4", "h264", "aac");
        with_subtitles.has_unsupported_preview_streams = true;
        assert_eq!(
            choose_preview_strategy(&with_subtitles),
            PreviewStrategy::Remux
        );
        let mut with_chapters = info("mov,mp4", "h264", "aac");
        with_chapters.chapter_count = 4;
        assert_eq!(
            choose_preview_strategy(&with_chapters),
            PreviewStrategy::Remux
        );
    }

    #[test]
    fn copies_compatible_video_and_transcodes_only_as_a_last_resort() {
        assert_eq!(
            choose_preview_strategy(&info("matroska,webm", "h264", "eac3")),
            PreviewStrategy::Remux
        );
        assert_eq!(
            choose_preview_strategy(&info("matroska,webm", "hevc", "eac3")),
            PreviewStrategy::Transcode,
            "WKWebView cannot play HEVC, even as hvc1 in MP4"
        );
        assert_eq!(
            choose_preview_strategy(&info("mov,mp4", "hevc", "aac")),
            PreviewStrategy::Transcode,
            "native HEVC MP4 still has to be transcoded for the webview"
        );
        assert_eq!(
            choose_preview_strategy(&info("matroska,webm", "av1", "opus")),
            PreviewStrategy::Transcode
        );
        let mut high_ten_h264 = info("matroska,webm", "h264", "aac");
        high_ten_h264.video_pixel_format = "yuv420p10le".into();
        assert_eq!(
            choose_preview_strategy(&high_ten_h264),
            PreviewStrategy::Transcode,
            "WebKit compatibility needs pixel-format checks, not only codec names"
        );
    }

    #[test]
    fn preview_stays_separate_from_export_and_strips_extra_streams() {
        let source = include_str!("proxy.rs");
        for flag in ["\"-map_chapters\"", "\"-dn\"", "\"-sn\""] {
            assert!(source.contains(flag), "preview must keep {flag}");
        }
        let export_source = include_str!("export.rs");
        assert!(
            !export_source.contains(PREVIEW_VERSION),
            "export must never read the disposable preview"
        );
    }
}
