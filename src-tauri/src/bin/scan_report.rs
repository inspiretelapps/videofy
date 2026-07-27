//! Measurement harness for detector calibration.
//!
//! Runs the detection passes over a movie without the GUI and reports what each
//! source produced, so thresholds can be chosen from observed behaviour instead
//! of guessed. Every threshold currently in the codebase is a guess; this is the
//! instrument for replacing them.
//!
//!   cargo run --bin scan_report -- MOVIE [options]
//!
//!     --annotations FILE   score detections against hand-annotated ranges
//!     --json FILE          write the full report as JSON
//!     --skip LIST          comma-separated: loudness,text,audio
//!     --labels N           show the top N YAMNet labels (default 25)
//!     --profanity TIER     off|strong|medium|mild (default medium)
//!     --no-verify          skip Whisper confirmation of subtitle mute timing
//!     --quiet              suppress per-pass progress
//!
//! Annotation file format (times are seconds or HH:MM:SS):
//!
//!   { "ranges": [ { "start": "00:12:30", "end": "00:13:05",
//!                   "categories": ["frightening"], "note": "bear attack" } ] }

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;
use videofy_lib::content::ContentEvent;
use videofy_lib::media::HeadlessHost;
use videofy_lib::text_analysis::ProfanityTier;
use videofy_lib::{analysis, audio_events, probe, text_analysis};

#[derive(Deserialize)]
struct Annotations {
    ranges: Vec<AnnotatedRange>,
}

#[derive(Deserialize, Clone)]
struct AnnotatedRange {
    start: TimeSpec,
    end: TimeSpec,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum TimeSpec {
    Seconds(f64),
    Clock(String),
}

impl TimeSpec {
    fn seconds(&self) -> Result<f64, String> {
        match self {
            TimeSpec::Seconds(value) => Ok(*value),
            TimeSpec::Clock(text) => {
                let parts: Vec<&str> = text.split(':').collect();
                let nums: Result<Vec<f64>, _> = parts
                    .iter()
                    .map(|p| p.replace(',', ".").parse::<f64>())
                    .collect();
                let nums = nums.map_err(|_| format!("bad timestamp {text:?}"))?;
                match nums.as_slice() {
                    [h, m, s] => Ok(h * 3600.0 + m * 60.0 + s),
                    [m, s] => Ok(m * 60.0 + s),
                    [s] => Ok(*s),
                    _ => Err(format!("bad timestamp {text:?}")),
                }
            }
        }
    }
}

#[derive(Serialize)]
struct SourceReport {
    source: String,
    events: usize,
    per_hour: f64,
    seconds: f64,
    by_category: BTreeMap<String, usize>,
    warnings: Vec<String>,
}

/// Compact per-event record. Written to the JSON report only — you need the
/// actual ranges to judge whether boundaries and timing are right, not just
/// counts.
#[derive(Serialize)]
struct EventSummary {
    start: f64,
    end: f64,
    category: String,
    severity: u8,
    confidence: f64,
    reason: String,
    source: String,
}

#[derive(Serialize)]
struct Report {
    movie: String,
    duration: f64,
    sources: Vec<SourceReport>,
    total_events: usize,
    total_per_hour: f64,
    events: Vec<EventSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    yamnet_labels: Option<Vec<audio_events::LabelStat>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scoring: Option<Scoring>,
}

#[derive(Serialize)]
struct Scoring {
    annotated_ranges: usize,
    /// Annotated ranges with at least one overlapping event of any category.
    hit_any: usize,
    /// Annotated ranges with an overlapping event of a matching category.
    hit_category: usize,
    recall_any: f64,
    recall_category: f64,
    /// Detected events that overlap some annotated range.
    events_on_target: usize,
    events_total: usize,
    precision: f64,
    /// Median absolute start-boundary error, in seconds, over matched ranges.
    median_boundary_error: f64,
    misses: Vec<String>,
}

fn hhmmss(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

fn category_of(event: &ContentEvent) -> String {
    format!("{:?}", event.category).to_lowercase()
}

fn summarize(
    source: &str,
    events: &[ContentEvent],
    duration: f64,
    seconds: f64,
    warnings: Vec<String>,
) -> SourceReport {
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        *by_category.entry(category_of(event)).or_insert(0) += 1;
    }
    SourceReport {
        source: source.to_string(),
        events: events.len(),
        per_hour: if duration > 0.0 {
            events.len() as f64 / (duration / 3600.0)
        } else {
            0.0
        },
        seconds,
        by_category,
        warnings,
    }
}

fn score(events: &[ContentEvent], ranges: &[AnnotatedRange]) -> Result<Scoring, String> {
    let mut hit_any = 0usize;
    let mut hit_category = 0usize;
    let mut boundary_errors: Vec<f64> = Vec::new();
    let mut misses = Vec::new();
    for range in ranges {
        let (start, end) = (range.start.seconds()?, range.end.seconds()?);
        let overlapping: Vec<&ContentEvent> = events
            .iter()
            .filter(|event| event.start < end && event.end > start)
            .collect();
        if overlapping.is_empty() {
            misses.push(format!(
                "{} - {}  [{}] {}",
                hhmmss(start),
                hhmmss(end),
                range.categories.join(", "),
                range.note.clone().unwrap_or_default()
            ));
            continue;
        }
        hit_any += 1;
        let wanted: Vec<String> = range.categories.iter().map(|c| c.to_lowercase()).collect();
        if wanted.is_empty() || overlapping.iter().any(|e| wanted.contains(&category_of(e))) {
            hit_category += 1;
        }
        let best = overlapping
            .iter()
            .map(|event| (event.start - start).abs())
            .fold(f64::INFINITY, f64::min);
        boundary_errors.push(best);
    }
    let on_target = events
        .iter()
        .filter(|event| {
            ranges
                .iter()
                .any(|range| match (range.start.seconds(), range.end.seconds()) {
                    (Ok(s), Ok(e)) => event.start < e && event.end > s,
                    _ => false,
                })
        })
        .count();
    boundary_errors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if boundary_errors.is_empty() {
        0.0
    } else {
        boundary_errors[boundary_errors.len() / 2]
    };
    Ok(Scoring {
        annotated_ranges: ranges.len(),
        hit_any,
        hit_category,
        recall_any: ratio(hit_any, ranges.len()),
        recall_category: ratio(hit_category, ranges.len()),
        events_on_target: on_target,
        events_total: events.len(),
        precision: ratio(on_target, events.len()),
        median_boundary_error: median,
        misses,
    })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("scan_report: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut movie = None;
    let mut annotations_path = None;
    let mut json_path = None;
    let mut skip: Vec<String> = Vec::new();
    let mut top_labels = 25usize;
    let mut profanity = ProfanityTier::Medium;
    let mut verify_mutes = true;
    let mut quiet = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--annotations" => {
                index += 1;
                annotations_path = args.get(index).cloned();
            }
            "--json" => {
                index += 1;
                json_path = args.get(index).cloned();
            }
            "--skip" => {
                index += 1;
                skip = args
                    .get(index)
                    .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
            }
            "--labels" => {
                index += 1;
                top_labels = args.get(index).and_then(|v| v.parse().ok()).unwrap_or(25);
            }
            "--profanity" => {
                index += 1;
                profanity = match args.get(index).map(String::as_str) {
                    Some("off") => ProfanityTier::Off,
                    Some("strong") => ProfanityTier::Strong,
                    Some("medium") => ProfanityTier::Medium,
                    Some("mild") => ProfanityTier::Mild,
                    other => {
                        return Err(format!(
                            "--profanity expects off|strong|medium|mild, got {other:?}"
                        ))
                    }
                };
            }
            "--no-verify" => verify_mutes = false,
            "--quiet" => quiet = true,
            other if !other.starts_with("--") => movie = Some(other.to_string()),
            other => return Err(format!("unknown option {other}")),
        }
        index += 1;
    }
    let movie =
        movie.ok_or("usage: scan_report MOVIE [--annotations F] [--json F] [--skip LIST]")?;
    let host = HeadlessHost::new(!quiet)?;
    let running = |name: &str| !skip.iter().any(|s| s == name);

    let info = probe::probe_sync(&movie)?;
    let duration = info.duration;
    println!("{}", info.file_name);
    println!(
        "  {} · {}x{} · {:.0} fps · {} audio track(s)",
        hhmmss(duration),
        info.width,
        info.height,
        info.fps,
        info.audio_tracks
    );
    for track in info.tracks.iter().filter(|t| t.kind == "subtitle") {
        println!(
            "  subtitle: {} {}{}{}",
            track.codec,
            track.language.clone().unwrap_or_else(|| "??".into()),
            if track.is_hearing_impaired {
                " SDH"
            } else {
                ""
            },
            if track.is_text {
                ""
            } else {
                " (not extractable)"
            }
        );
    }
    println!();

    let mut sources: Vec<SourceReport> = Vec::new();
    let mut all_events: Vec<ContentEvent> = Vec::new();
    let mut yamnet_labels = None;

    if running("loudness") {
        let started = Instant::now();
        match analysis::analyze_with_host(&host, &movie, duration, 0.5) {
            Ok(result) => {
                sources.push(summarize(
                    "loudness",
                    &result.events,
                    duration,
                    started.elapsed().as_secs_f64(),
                    Vec::new(),
                ));
                all_events.extend(result.events);
            }
            Err(error) => sources.push(summarize(
                "loudness",
                &[],
                duration,
                started.elapsed().as_secs_f64(),
                vec![error],
            )),
        }
        if !quiet {
            eprintln!();
        }
    }

    if running("text") {
        let started = Instant::now();
        match text_analysis::analyze_with_host(&host, &movie, duration, profanity, verify_mutes) {
            Ok(result) => {
                let mut warnings = result.warnings.clone();
                warnings.push(format!(
                    "source: {} ({} cues)",
                    result.source, result.cue_count
                ));
                sources.push(summarize(
                    "text",
                    &result.events,
                    duration,
                    started.elapsed().as_secs_f64(),
                    warnings,
                ));
                all_events.extend(result.events);
            }
            Err(error) => sources.push(summarize(
                "text",
                &[],
                duration,
                started.elapsed().as_secs_f64(),
                vec![error],
            )),
        }
        if !quiet {
            eprintln!();
        }
    }

    if running("audio") {
        let started = Instant::now();
        match audio_events::analyze_with_host(&host, &movie, duration, true) {
            Ok((result, stats)) => {
                sources.push(summarize(
                    "audio (YAMNet)",
                    &result.events,
                    duration,
                    started.elapsed().as_secs_f64(),
                    result.warnings.clone(),
                ));
                all_events.extend(result.events);
                yamnet_labels = stats.map(|s| s.labels);
            }
            Err(error) => sources.push(summarize(
                "audio (YAMNet)",
                &[],
                duration,
                started.elapsed().as_secs_f64(),
                vec![error],
            )),
        }
        if !quiet {
            eprintln!();
        }
    }

    println!(
        "{:<16} {:>7} {:>10} {:>8}",
        "SOURCE", "EVENTS", "PER HOUR", "SECONDS"
    );
    for report in &sources {
        println!(
            "{:<16} {:>7} {:>10.1} {:>8.1}",
            report.source, report.events, report.per_hour, report.seconds
        );
        for (category, count) in &report.by_category {
            println!("    {category:<20} {count:>5}");
        }
        for warning in &report.warnings {
            println!("    ! {warning}");
        }
    }
    let total = all_events.len();
    let per_hour = if duration > 0.0 {
        total as f64 / (duration / 3600.0)
    } else {
        0.0
    };
    println!("{:<16} {:>7} {:>10.1}", "TOTAL", total, per_hour);
    println!();

    if let Some(labels) = &yamnet_labels {
        println!("YAMNet label scores — frames at or above each threshold");
        println!(
            "{:<30} {:>7} {:>7} {:>7} {:>7} {:>7}  {:>5}",
            "LABEL", ">=0.2", ">=0.3", ">=0.4", ">=0.5", ">=0.7", "MAX"
        );
        for stat in labels.iter().take(top_labels) {
            println!(
                "{:<30} {:>7} {:>7} {:>7} {:>7} {:>7}  {:>5.2}",
                stat.label,
                stat.buckets[1],
                stat.buckets[2],
                stat.buckets[3],
                stat.buckets[4],
                stat.buckets[6],
                stat.max_score
            );
        }
        println!();
        println!("Pick thresholds where the count drops to what you would actually review.");
        println!();
    }

    let scoring = match &annotations_path {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let annotations: Annotations =
                serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            let result = score(&all_events, &annotations.ranges)?;
            println!(
                "Scored against {} annotated range(s)",
                result.annotated_ranges
            );
            println!(
                "  recall (any category):      {:>5.1}%  ({}/{})",
                result.recall_any * 100.0,
                result.hit_any,
                result.annotated_ranges
            );
            println!(
                "  recall (matching category): {:>5.1}%  ({}/{})",
                result.recall_category * 100.0,
                result.hit_category,
                result.annotated_ranges
            );
            println!(
                "  precision (events on target): {:>3.1}%  ({}/{})",
                result.precision * 100.0,
                result.events_on_target,
                result.events_total
            );
            println!(
                "  median start-boundary error: {:.1}s",
                result.median_boundary_error
            );
            if !result.misses.is_empty() {
                println!("  missed entirely:");
                for miss in &result.misses {
                    println!("    {miss}");
                }
            }
            println!();
            Some(result)
        }
        None => None,
    };

    let mut listed: Vec<EventSummary> = all_events
        .iter()
        .map(|event| EventSummary {
            start: event.start,
            end: event.end,
            category: category_of(event),
            severity: event.severity,
            confidence: event.confidence,
            reason: event.reason.clone(),
            source: event.source_key.clone(),
        })
        .collect();
    listed.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let report = Report {
        movie: info.file_name.clone(),
        duration,
        sources,
        total_events: total,
        total_per_hour: per_hour,
        events: listed,
        yamnet_labels,
        scoring,
    };
    if let Some(path) = json_path {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{path}: {e}"))?;
        println!("Wrote {path}");
    }
    Ok(())
}
