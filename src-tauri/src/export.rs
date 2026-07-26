use crate::media;
use serde::{Deserialize, Serialize};
use std::io::Write;
use tauri::Emitter;

#[derive(Deserialize, Clone, Copy)]
pub struct Cut {
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub out_path: String,
    pub kept_duration: f64,
    pub removed_duration: f64,
    pub size_bytes: u64,
    pub segments: usize,
}

#[derive(Clone, Copy)]
struct Keep {
    inpoint: f64,
    outpoint: f64,
}

/// Merge overlapping cuts, complement them into keep-segments, and snap each
/// keep-segment start forward to the next keyframe. Snapping forward means we
/// only ever remove slightly MORE than marked — never leak scary frames back
/// in — and every segment starts clean for lossless stream copy.
fn plan_segments(cuts: &[Cut], keyframes: &[f64], duration: f64) -> Result<Vec<Keep>, String> {
    let mut cuts: Vec<Cut> = cuts
        .iter()
        .map(|c| Cut {
            start: c.start.clamp(0.0, duration),
            end: c.end.clamp(0.0, duration),
        })
        .filter(|c| c.end > c.start)
        .collect();
    cuts.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let mut merged: Vec<Cut> = Vec::new();
    for c in cuts {
        match merged.last_mut() {
            Some(prev) if c.start <= prev.end + 0.05 => prev.end = prev.end.max(c.end),
            _ => merged.push(c),
        }
    }

    let mut keeps: Vec<Keep> = Vec::new();
    let mut cursor = 0.0;
    for c in &merged {
        if c.start > cursor {
            keeps.push(Keep { inpoint: cursor, outpoint: c.start });
        }
        cursor = cursor.max(c.end);
    }
    if cursor < duration {
        keeps.push(Keep { inpoint: cursor, outpoint: duration });
    }

    let mut snapped: Vec<Keep> = Vec::new();
    for k in keeps {
        let inpoint = if k.inpoint <= 0.001 {
            0.0
        } else {
            // first keyframe at/after the requested start; epsilon guards float
            // error so the demuxer's "keyframe at or before inpoint" is this one
            match keyframes
                .iter()
                .find(|&&kf| kf >= k.inpoint - 0.002)
                .copied()
            {
                Some(kf) => kf + 0.001,
                None => continue, // no keyframe left before end of file
            }
        };
        if k.outpoint - inpoint > 0.2 {
            snapped.push(Keep { inpoint, outpoint: k.outpoint });
        }
    }
    if snapped.is_empty() {
        return Err("nothing left to export — the cuts remove the whole movie".into());
    }
    Ok(snapped)
}

fn concat_escape(path: &str) -> String {
    path.replace('\'', "'\\''")
}

#[tauri::command]
pub async fn export_video(
    app: tauri::AppHandle,
    path: String,
    out_path: String,
    cuts: Vec<Cut>,
    keyframes: Vec<f64>,
    duration: f64,
) -> Result<ExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let keeps = plan_segments(&cuts, &keyframes, duration)?;
        let kept: f64 = keeps.iter().map(|k| k.outpoint - k.inpoint).sum();

        let dir = media::cache_dir_for(&app, &path)?;
        let list_path = dir.join("export.ffconcat");
        {
            let mut f = std::fs::File::create(&list_path).map_err(|e| e.to_string())?;
            writeln!(f, "ffconcat version 1.0").map_err(|e| e.to_string())?;
            for k in &keeps {
                writeln!(f, "file '{}'", concat_escape(&path)).map_err(|e| e.to_string())?;
                if k.inpoint > 0.0 {
                    writeln!(f, "inpoint {:.6}", k.inpoint).map_err(|e| e.to_string())?;
                }
                writeln!(f, "outpoint {:.6}", k.outpoint).map_err(|e| e.to_string())?;
            }
        }

        let list_str = list_path.to_string_lossy().to_string();
        let ffmpeg = media::ffmpeg_path();
        let args = [
            "-y", "-v", "error", "-nostats", "-progress", "pipe:1",
            "-f", "concat", "-safe", "0",
            "-i", &list_str,
            "-map", "0:v:0", "-map", "0:a?", "-map", "0:s?",
            "-c", "copy",
            "-avoid_negative_ts", "make_zero",
            "-map_chapters", "-1",
            &out_path,
        ];
        let mut child = media::spawn(&ffmpeg, &args)?;
        let stderr_drain = media::drain_stderr(&mut child);
        if let Some(stdout) = child.stdout.take() {
            let reader = std::io::BufReader::new(stdout);
            media::read_progress(reader, |t| {
                let pct = if kept > 0.0 { (t / kept * 100.0).min(100.0) } else { 0.0 };
                let _ = app.emit("export-progress", serde_json::json!({ "t": t, "pct": pct }));
            });
        }
        media::wait_checked(child, "export", stderr_drain)?;

        let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
        let _ = app.emit("export-progress", serde_json::json!({ "t": kept, "pct": 100.0 }));
        Ok(ExportResult {
            out_path,
            kept_duration: kept,
            removed_duration: (duration - kept).max(0.0),
            size_bytes: size,
            segments: keeps.len(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_merges_and_snaps() {
        let cuts = vec![
            Cut { start: 10.0, end: 15.0 },
            Cut { start: 14.0, end: 20.0 },
            Cut { start: 50.0, end: 55.0 },
        ];
        let keyframes: Vec<f64> = (0..30).map(|i| i as f64 * 4.170).collect();
        let keeps = plan_segments(&cuts, &keyframes, 100.0).unwrap();
        assert_eq!(keeps.len(), 3);
        assert_eq!(keeps[0].inpoint, 0.0);
        assert!((keeps[0].outpoint - 10.0).abs() < 1e-9);
        // 20.0 snaps forward to keyframe 20.85
        assert!(keeps[1].inpoint > 20.0 && keeps[1].inpoint < 20.86);
        assert!((keeps[1].outpoint - 50.0).abs() < 1e-9);
        // 55.0 snaps forward to keyframe 58.38
        assert!(keeps[2].inpoint > 55.0 && keeps[2].inpoint < 58.39);
    }

    #[test]
    fn plan_rejects_total_removal() {
        let cuts = vec![Cut { start: 0.0, end: 100.0 }];
        assert!(plan_segments(&cuts, &[0.0], 100.0).is_err());
    }
}
