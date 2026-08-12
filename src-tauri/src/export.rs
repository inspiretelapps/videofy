use crate::{media, probe};
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
    pub muted_duration: f64,
    pub size_bytes: u64,
    pub segments: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct Keep {
    inpoint: f64,
    outpoint: f64,
}

struct SegmentPlan {
    keeps: Vec<Keep>,
    warnings: Vec<String>,
}

/// Merge overlapping cuts, complement them into keep-segments, and snap each
/// keep-segment start forward to the next keyframe. Snapping forward means we
/// only ever remove slightly MORE than marked — never leak scary frames back
/// in — and every segment starts clean for lossless stream copy.
fn plan_segments(cuts: &[Cut], keyframes: &[f64], duration: f64) -> Result<SegmentPlan, String> {
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
            keeps.push(Keep {
                inpoint: cursor,
                outpoint: c.start,
            });
        }
        cursor = cursor.max(c.end);
    }
    if cursor < duration {
        keeps.push(Keep {
            inpoint: cursor,
            outpoint: duration,
        });
    }

    let mut snapped: Vec<Keep> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
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
                None => {
                    warnings.push(format!(
                        "Dropped {:.1}s–{:.1}s: no keyframe remains after that point, \
                         so it cannot be copied losslessly.",
                        k.inpoint, k.outpoint
                    ));
                    continue;
                }
            }
        };
        if k.outpoint - inpoint > 0.2 {
            snapped.push(Keep {
                inpoint,
                outpoint: k.outpoint,
            });
        }
    }
    if snapped.is_empty() {
        return Err("nothing left to export — the cuts remove the whole movie".into());
    }
    Ok(SegmentPlan {
        keeps: snapped,
        warnings,
    })
}

fn map_mutes_to_output(mutes: &[Cut], keeps: &[Keep]) -> Vec<Cut> {
    let mut mapped = Vec::new();
    let mut output_cursor = 0.0;
    for keep in keeps {
        for mute in mutes {
            let start = mute.start.max(keep.inpoint);
            let end = mute.end.min(keep.outpoint);
            if end > start {
                mapped.push(Cut {
                    start: output_cursor + start - keep.inpoint,
                    end: output_cursor + end - keep.inpoint,
                });
            }
        }
        output_cursor += keep.outpoint - keep.inpoint;
    }
    mapped.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let mut merged: Vec<Cut> = Vec::new();
    for mute in mapped {
        match merged.last_mut() {
            Some(previous) if mute.start <= previous.end + 0.04 => {
                previous.end = previous.end.max(mute.end)
            }
            _ => merged.push(mute),
        }
    }
    merged
}

/// Ducked level for a muted range. Full silence leaves an obvious hole that
/// makes a child ask what the word was; -30 dB is inaudible in context but
/// keeps the room tone, so the edit reads as nothing at all.
const MUTE_LEVEL: f64 = 0.0316;
/// Ramp on each side of a muted range. An instantaneous gain step clicks; the
/// ramp sits outside the range so the full duck still covers the whole word.
/// The volume filter re-evaluates per audio frame (~21 ms at 48 kHz), so this
/// is a few steps rather than a true glide — enough to remove the click.
const MUTE_RAMP: f64 = 0.05;

/// Builds a time-varying gain as the product of one attenuation per muted
/// range, so overlapping ramps compose instead of fighting.
fn mute_filter(mutes: &[Cut]) -> String {
    let gain = mutes
        .iter()
        .map(|mute| {
            let from = (mute.start - MUTE_RAMP).max(0.0);
            let to = mute.end + MUTE_RAMP;
            format!(
                "(1-(1-{MUTE_LEVEL})*clip(min((t-{from:.3})/{MUTE_RAMP}\\,({to:.3}-t)/{MUTE_RAMP})\\,0\\,1))"
            )
        })
        .collect::<Vec<_>>()
        .join("*");
    format!("volume=eval=frame:volume='{gain}'")
}

/// AAC bitrate scaled to channel count. A 5.1 mix squeezed into 192k is a
/// noticeable downgrade applied to the whole film for the sake of a few words.
fn aac_bitrate(channels: u32) -> String {
    match channels {
        0..=2 => "192k",
        3..=6 => "384k",
        _ => "512k",
    }
    .to_string()
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
    mutes: Vec<Cut>,
    keyframes: Vec<f64>,
    duration: f64,
) -> Result<ExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = media::cache_dir_for(&app, &path)?;
        let _guard = media::JobGuard::acquire(format!("export:{}", dir.display()))?;
        let plan = plan_segments(&cuts, &keyframes, duration)?;
        let keeps = plan.keeps;
        let warnings = plan.warnings;
        let kept: f64 = keeps.iter().map(|k| k.outpoint - k.inpoint).sum();
        let output_mutes = map_mutes_to_output(&mutes, &keeps);
        let muted_duration: f64 = output_mutes.iter().map(|mute| mute.end - mute.start).sum();

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
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-v".into(),
            "error".into(),
            "-nostats".into(),
            "-progress".into(),
            "pipe:1".into(),
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list_str,
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a?".into(),
            "-map".into(),
            "0:s?".into(),
        ];
        if output_mutes.is_empty() {
            args.extend(["-c".into(), "copy".into()]);
        } else {
            let channels = probe::probe_sync(&path)
                .map(|info| {
                    info.tracks
                        .iter()
                        .filter(|track| track.kind == "audio")
                        .map(|track| track.channels)
                        .max()
                        .unwrap_or(2)
                })
                .unwrap_or(2);
            args.extend([
                "-c:v".into(),
                "copy".into(),
                "-c:s".into(),
                "copy".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                aac_bitrate(channels),
                "-af".into(),
                mute_filter(&output_mutes),
            ]);
        }
        args.extend([
            "-avoid_negative_ts".into(),
            "make_zero".into(),
            "-map_chapters".into(),
            "-1".into(),
            out_path.clone(),
        ]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut child = media::spawn(&ffmpeg, &arg_refs)?;
        let stderr_drain = media::drain_stderr(&mut child);
        if let Some(stdout) = child.stdout.take() {
            let reader = std::io::BufReader::new(stdout);
            media::read_progress(reader, |t| {
                let pct = if kept > 0.0 {
                    (t / kept * 100.0).min(100.0)
                } else {
                    0.0
                };
                let _ = app.emit("export-progress", serde_json::json!({ "t": t, "pct": pct }));
            });
        }
        media::wait_checked(child, "export", stderr_drain)?;

        let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
        let _ = app.emit(
            "export-progress",
            serde_json::json!({ "t": kept, "pct": 100.0 }),
        );
        Ok(ExportResult {
            out_path,
            kept_duration: kept,
            removed_duration: (duration - kept).max(0.0),
            muted_duration,
            size_bytes: size,
            segments: keeps.len(),
            warnings,
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
            Cut {
                start: 10.0,
                end: 15.0,
            },
            Cut {
                start: 14.0,
                end: 20.0,
            },
            Cut {
                start: 50.0,
                end: 55.0,
            },
        ];
        let keyframes: Vec<f64> = (0..30).map(|i| i as f64 * 4.170).collect();
        let keeps = plan_segments(&cuts, &keyframes, 100.0).unwrap().keeps;
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
    fn mute_ducks_rather_than_cutting_a_hole() {
        let filter = mute_filter(&[Cut {
            start: 10.0,
            end: 11.0,
        }]);
        assert!(filter.contains("eval=frame"), "gain must vary over time");
        assert!(filter.contains("0.0316"), "should duck, not silence");
        assert!(
            !filter.contains("volume=0:"),
            "hard gate is what we replaced"
        );
        // Ramps sit outside the range so the whole word stays fully ducked.
        assert!(filter.contains("9.950") && filter.contains("11.050"));
        // Commas inside the expression must reach ffmpeg escaped, or the
        // filtergraph parser reads them as filter separators and the export
        // fails. Verified against ffmpeg 8: this exact string is accepted.
        assert!(
            filter.contains("/0.05\\,"),
            "expression commas must be escaped"
        );
    }

    #[test]
    fn multiple_mutes_compose_as_a_product() {
        let filter = mute_filter(&[
            Cut {
                start: 1.0,
                end: 2.0,
            },
            Cut {
                start: 5.0,
                end: 6.0,
            },
        ]);
        assert_eq!(filter.matches("clip(").count(), 2);
        assert!(filter.contains(")*("));
    }

    #[test]
    fn bitrate_scales_with_channel_count() {
        assert_eq!(aac_bitrate(2), "192k");
        assert_eq!(aac_bitrate(6), "384k");
        assert_eq!(aac_bitrate(8), "512k");
    }

    #[test]
    fn plan_rejects_total_removal() {
        let cuts = vec![Cut {
            start: 0.0,
            end: 100.0,
        }];
        assert!(plan_segments(&cuts, &[0.0], 100.0).is_err());
    }

    #[test]
    fn plan_warns_when_a_keep_has_no_later_keyframe() {
        let cuts = vec![Cut {
            start: 10.0,
            end: 20.0,
        }];
        let plan = plan_segments(&cuts, &[0.0, 4.0, 8.0], 100.0).unwrap();
        assert_eq!(plan.keeps.len(), 1);
        assert_eq!(plan.keeps[0].inpoint, 0.0);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("20"));
    }

    #[test]
    fn mute_ranges_follow_removed_segments() {
        let keeps = vec![
            Keep {
                inpoint: 0.0,
                outpoint: 10.0,
            },
            Keep {
                inpoint: 20.0,
                outpoint: 40.0,
            },
        ];
        let mapped = map_mutes_to_output(
            &[
                Cut {
                    start: 5.0,
                    end: 6.0,
                },
                Cut {
                    start: 25.0,
                    end: 27.0,
                },
            ],
            &keeps,
        );
        assert_eq!(mapped.len(), 2);
        assert!((mapped[1].start - 15.0).abs() < 0.001);
        assert!((mapped[1].end - 17.0).abs() < 0.001);
    }
}
