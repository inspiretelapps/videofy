use crate::{media, probe};
use tauri::Emitter;

/// Generates a low-res H.264/AAC preview the webview can always play,
/// regardless of the source container/codec. Edits are made against this
/// proxy's timeline but applied to the untouched original on export.
#[tauri::command]
pub async fn generate_proxy(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    source_height: u32,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = media::cache_dir_for(&app, &path)?;
        let _guard = media::JobGuard::acquire(format!("proxy:{}", dir.display()))?;
        let proxy = dir.join("proxy-v3.mp4");
        if proxy.exists() {
            return Ok(proxy.to_string_lossy().to_string());
        }
        // clear tmp files orphaned by a previous crash or force-quit
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("proxy-v3.tmp")
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let tmp = dir.join(format!("proxy-v3.tmp.{}.mp4", std::process::id()));

        let target_height = source_height.min(540);
        // force even dimensions for h264
        let target_height = target_height - (target_height % 2);
        let scale = format!("scale=-2:{}", target_height.max(2));
        let tmp_str = tmp.to_string_lossy().to_string();

        let ffmpeg = media::ffmpeg_path();
        let info = probe::probe_sync(&path)?;
        let audio_map = probe::preferred_audio_stream(&info)
            .map(|index| format!("0:{index}?"))
            .unwrap_or_else(|| "0:a:0?".into());
        let args = [
            "-y",
            "-v",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
            "-hwaccel",
            "videotoolbox",
            "-i",
            &path,
            "-map",
            "0:v:0",
            "-map",
            &audio_map,
            "-vf",
            &scale,
            "-c:v",
            "h264_videotoolbox",
            "-b:v",
            "2000k",
            "-allow_sw",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-ac",
            "2",
            // Strip everything that is not picture or sound. ffmpeg's mp4
            // muxer copies source chapters into a `text` track, which left the
            // proxy with a third stream the webview had to make sense of —
            // and WebKit responded by playing the video without any audio.
            // export.rs already dropped chapters for the same reason.
            "-map_chapters",
            "-1",
            "-dn",
            "-sn",
            "-movflags",
            "+faststart",
            &tmp_str,
        ];
        let mut child = media::spawn(&ffmpeg, &args)?;
        let stderr_drain = media::drain_stderr(&mut child);

        if let Some(stdout) = child.stdout.take() {
            let reader = std::io::BufReader::new(stdout);
            media::read_progress(reader, |t| {
                let pct = if duration > 0.0 {
                    (t / duration * 100.0).min(100.0)
                } else {
                    0.0
                };
                let _ = app.emit("proxy-progress", serde_json::json!({ "t": t, "pct": pct }));
            });
        }
        media::wait_checked(child, "proxy generation", stderr_drain)?;
        std::fs::rename(&tmp, &proxy).map_err(|e| e.to_string())?;
        let _ = app.emit(
            "proxy-progress",
            serde_json::json!({ "t": duration, "pct": 100.0 }),
        );
        Ok(proxy.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    /// The proxy carried a chapter `text` track for as long as this command
    /// existed, and WebKit answered by playing the picture with no sound.
    /// These flags are load-bearing, not tidiness.
    #[test]
    fn strips_non_audiovisual_streams() {
        let source = include_str!("proxy.rs");
        for flag in ["\"-map_chapters\"", "\"-dn\"", "\"-sn\""] {
            assert!(
                source.contains(flag),
                "proxy must keep {flag}: a stray stream silences audio in the webview"
            );
        }
    }
}
