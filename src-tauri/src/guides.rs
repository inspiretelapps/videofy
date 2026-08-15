use crate::content::{
    merge_events, stable_id, ContentCategory, ContentEvent, EventAction, Evidence,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GuideResult {
    pub provider: String,
    pub title: Option<String>,
    pub events: Vec<ContentEvent>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn import_timing_file(path: String, offset: Option<f64>) -> Result<GuideResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let extension = std::path::Path::new(&path)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let offset = offset.unwrap_or(0.0);
        let events = if extension == "skp" {
            parse_skip_file(&text, offset)
        } else {
            parse_caption_file(&text, offset)
        };
        if events.is_empty() {
            return Err("No usable timestamp ranges were found in this file.".into());
        }
        Ok(GuideResult {
            provider: if extension == "skp" {
                "Skip file".into()
            } else {
                "Timestamp subtitles".into()
            },
            title: std::path::Path::new(&path)
                .file_stem()
                .map(|v| v.to_string_lossy().to_string()),
            events: merge_events(events),
            warnings: Vec::new(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DddItem {
    id: u64,
    name: String,
    release_year: Option<i32>,
    topic_item_stats: Option<Vec<DddTopicStat>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DddTopicStat {
    topic_id: u64,
    topic_name: String,
    yes_sum: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DddRating {
    topic_id: u64,
    yes: Option<i32>,
    trigger_description: Option<String>,
    cue_description: Option<String>,
    position1: Option<f64>,
    position2: Option<f64>,
    position3: Option<f64>,
    safe_position1: Option<f64>,
    safe_position2: Option<f64>,
    safe_position3: Option<f64>,
    is_scene_alert: Option<bool>,
}

#[tauri::command]
pub async fn lookup_content_guide(
    api_key: String,
    title: String,
    year: Option<i32>,
) -> Result<GuideResult, String> {
    tauri::async_runtime::spawn_blocking(move || lookup_ddd(&api_key, &title, year))
        .await
        .map_err(|e| e.to_string())?
}

fn lookup_ddd(api_key: &str, title: &str, year: Option<i32>) -> Result<GuideResult, String> {
    if api_key.trim().is_empty() {
        return Err("A DoesTheDogDie.com API key is required.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent("Videofy/0.1.1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut search_url = format!(
        "https://www.doesthedogdie.com/api/v3/items?name={}",
        urlencoding::encode(title)
    );
    if let Some(year) = year {
        search_url.push_str(&format!("&releaseYear={year}"));
    }
    let search: Vec<DddItem> = client
        .get(&search_url)
        .header("X-API-KEY", api_key)
        .send()
        .map_err(|e| format!("Guide search failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Guide search failed: {e}"))?
        .json()
        .map_err(|e| format!("Unexpected guide response: {e}"))?;
    let found = search
        .into_iter()
        .min_by_key(|item| {
            let name_penalty = if item.name.eq_ignore_ascii_case(title) {
                0
            } else {
                2
            };
            let year_penalty = match (year, item.release_year) {
                (Some(a), Some(b)) => (a - b).unsigned_abs(),
                _ => 1,
            };
            name_penalty + year_penalty
        })
        .ok_or_else(|| format!("No guide entry was found for “{title}”."))?;

    let detail_url = format!("https://www.doesthedogdie.com/api/v3/items/{}", found.id);
    let detail: DddItem = client
        .get(&detail_url)
        .header("X-API-KEY", api_key)
        .send()
        .map_err(|e| format!("Guide details failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Guide details failed: {e}"))?
        .json()
        .map_err(|e| format!("Unexpected guide details: {e}"))?;

    let mut events = Vec::new();
    let mut warnings = Vec::new();
    let relevant: HashMap<_, _> = detail
        .topic_item_stats
        .unwrap_or_default()
        .into_iter()
        .filter(|stat| stat.yes_sum.unwrap_or(0) > 0 && topic_category(&stat.topic_name).is_some())
        .map(|stat| (stat.topic_id, stat))
        .collect();
    if !relevant.is_empty() {
        let ratings_url = format!(
            "https://www.doesthedogdie.com/api/v3/items/{}/ratings",
            found.id
        );
        let response = client
            .get(&ratings_url)
            .header("X-API-KEY", api_key)
            .send()
            .map_err(|e| format!("Guide ratings failed: {e}"))?;
        if response.status().as_u16() == 403 {
            warnings.push(
                "This API key cannot read timestamped ratings. The free tier includes community timestamps — check the key on DoesTheDogDie.com/api.".into(),
            );
        } else {
            let ratings: Vec<DddRating> = response
                .error_for_status()
                .map_err(|e| format!("Guide ratings failed: {e}"))?
                .json()
                .map_err(|e| format!("Unexpected guide ratings: {e}"))?;
            for rating in ratings {
                let Some(stat) = relevant.get(&rating.topic_id) else {
                    continue;
                };
                if rating.yes.unwrap_or(1) == 0 {
                    continue;
                }
                let Some(start) = hms(rating.position1, rating.position2, rating.position3) else {
                    continue;
                };
                let end = hms(
                    rating.safe_position1,
                    rating.safe_position2,
                    rating.safe_position3,
                )
                .filter(|end| *end > start)
                .unwrap_or(start + 6.0);
                let category = topic_category(&stat.topic_name).unwrap();
                let description = rating
                    .trigger_description
                    .clone()
                    .unwrap_or_else(|| stat.topic_name.clone());
                let confidence = if rating.is_scene_alert.unwrap_or(false) {
                    0.99
                } else {
                    0.86
                };
                events.push(ContentEvent {
                    id: stable_id("does-the-dog-die", category, start, end, &description),
                    start,
                    end,
                    peak_time: start,
                    category,
                    severity: 3,
                    confidence,
                    reason: description.clone(),
                    suggested_action: EventAction::Cut,
                    evidence: vec![Evidence {
                        source: "DoesTheDogDie.com".into(),
                        label: stat.topic_name.clone(),
                        detail: rating.cue_description.clone(),
                        confidence,
                    }],
                    source_key: "does-the-dog-die".into(),
                });
            }
        }
    }
    if events.is_empty() && warnings.is_empty() {
        warnings.push(
            "The title has content warnings, but no timestamped entries were available.".into(),
        );
    }
    Ok(GuideResult {
        provider: "DoesTheDogDie.com".into(),
        title: Some(found.name),
        events: merge_events(events),
        warnings,
    })
}

fn hms(h: Option<f64>, m: Option<f64>, s: Option<f64>) -> Option<f64> {
    if h.is_none() && m.is_none() && s.is_none() {
        None
    } else {
        Some(h.unwrap_or(0.0) * 3600.0 + m.unwrap_or(0.0) * 60.0 + s.unwrap_or(0.0))
    }
}

fn parse_caption_file(input: &str, offset: f64) -> Vec<ContentEvent> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split("\n\n")
        .filter_map(|block| {
            let mut lines = block.lines().filter(|line| !line.trim().is_empty());
            let first = lines.next()?;
            let time_line = if first.contains("-->") {
                first
            } else {
                lines.next()?
            };
            let (cue_start, cue_end) = parse_range(time_line)?;
            let reason = lines.collect::<Vec<_>>().join(" ");
            let lower = reason.to_lowercase();
            let category = topic_category(&lower).unwrap_or(ContentCategory::Frightening);
            // Where's The Jump warning subtitles appear roughly five seconds
            // before the impact. Use the cue end as the likely event point.
            let jump_warning = lower.contains("jump") || lower.contains("scare");
            let start = if jump_warning {
                cue_end - 0.5
            } else {
                cue_start
            };
            let end = if jump_warning { cue_end + 2.5 } else { cue_end };
            let start = (start + offset).max(0.0);
            let end = (end + offset).max(start + 0.1);
            Some(ContentEvent {
                id: stable_id("timing-file", category, start, end, &reason),
                start,
                end,
                peak_time: if jump_warning {
                    cue_end + offset
                } else {
                    start
                },
                category,
                severity: if jump_warning { 2 } else { 3 },
                confidence: 0.9,
                reason: if reason.trim().is_empty() {
                    "Imported timestamp warning".into()
                } else {
                    reason.trim().to_string()
                },
                suggested_action: EventAction::Cut,
                evidence: vec![Evidence {
                    source: "imported timing file".into(),
                    label: "Published/user-supplied timestamp".into(),
                    detail: None,
                    confidence: 0.9,
                }],
                source_key: "timing-file".into(),
            })
        })
        .collect()
}

fn parse_skip_file(input: &str, offset: f64) -> Vec<ContentEvent> {
    let lines: Vec<&str> = input.lines().collect();
    let mut events = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].contains("-->") {
            index += 1;
            continue;
        }
        let Some((raw_start, raw_end)) = parse_range(lines[index]) else {
            index += 1;
            continue;
        };
        let label = lines
            .get(index + 1)
            .copied()
            .unwrap_or("Imported skip")
            .trim();
        let lower = label.to_lowercase();
        let category = topic_category(&lower).unwrap_or(ContentCategory::Disturbing);
        let severity = if lower.split_whitespace().any(|v| v == "3") {
            3
        } else if lower.split_whitespace().any(|v| v == "2") {
            2
        } else {
            1
        };
        let action = if lower.contains("audio") || lower.contains("sound") {
            EventAction::Mute
        } else {
            EventAction::Cut
        };
        let start = (raw_start + offset).max(0.0);
        let end = (raw_end + offset).max(start + 0.1);
        events.push(ContentEvent {
            id: stable_id("skip-file", category, start, end, label),
            start,
            end,
            peak_time: start,
            category,
            severity,
            confidence: 0.94,
            reason: label.to_string(),
            suggested_action: action,
            evidence: vec![Evidence {
                source: "imported skip file".into(),
                label: label.into(),
                detail: None,
                confidence: 0.94,
            }],
            source_key: "skip-file".into(),
        });
        index += 2;
    }
    events
}

fn parse_range(line: &str) -> Option<(f64, f64)> {
    let mut parts = line.split("-->");
    Some((
        parse_time(parts.next()?.trim())?,
        parse_time(parts.next()?.split_whitespace().next()?)?,
    ))
}

fn parse_time(value: &str) -> Option<f64> {
    let normalized = value.replace(',', ".");
    let parts: Vec<_> = normalized.trim().split(':').collect();
    match parts.as_slice() {
        [h, m, s] => Some(
            h.parse::<f64>().ok()? * 3600.0
                + m.parse::<f64>().ok()? * 60.0
                + s.parse::<f64>().ok()?,
        ),
        [m, s] => Some(m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?),
        _ => None,
    }
}

fn topic_category(topic: &str) -> Option<ContentCategory> {
    let topic = topic.to_lowercase();
    if contains(
        &topic,
        &[
            "sexual assault",
            "sexually assaulted",
            "rape",
            "pedoph",
            "minor sexualized",
        ],
    ) {
        Some(ContentCategory::Sexual)
    } else if contains(
        &topic,
        &[
            "jump scare",
            "scream",
            "ghost",
            "demon",
            "monster",
            "fright",
            "sudden loud",
            "creepy",
            "nightmare",
            "possess",
            "clown",
            "spider",
            "snake",
        ],
    ) {
        Some(ContentCategory::Frightening)
    } else if contains(
        &topic,
        &[
            "violence",
            "blood",
            "gore",
            "gun",
            "stab",
            "torture",
            "death",
            "dies",
            "killed",
            "body harm",
            "assault",
            "car crash",
            "mutilat",
            "injury",
            "body horror",
            "cannibal",
            "animal abused",
            "animal die",
            "animal death",
            "pet die",
            "dog die",
            "cat die",
            "horse die",
        ],
    ) {
        Some(ContentCategory::Violence)
    } else if contains(&topic, &["nudity", "nude", "topless"]) {
        Some(ContentCategory::Nudity)
    } else if contains(
        &topic,
        &[
            "sex",
            "intimacy",
            "sexual",
            "kissing",
            "pedoph",
            "minor sexualized",
        ],
    ) {
        Some(ContentCategory::Sexual)
    } else if contains(&topic, &["drug", "alcohol", "smoking", "overdose"]) {
        Some(ContentCategory::Substances)
    } else if contains(
        &topic,
        &["profanity", "language", "slur", "swearing", "curse word"],
    ) {
        Some(ContentCategory::Language)
    } else if contains(&topic, &["bully", "abuse", "cruel", "assault"]) {
        Some(ContentCategory::Bullying)
    } else if contains(
        &topic,
        &[
            "suicide",
            "self-harm",
            "disturb",
            "kidnap",
            "child peril",
            "child abused",
            "kid die",
            "corpse",
            "vomit",
            "miscarriage",
            "eating disorder",
            "terminal illness",
        ],
    ) {
        Some(ContentCategory::Disturbing)
    } else {
        None
    }
}

fn contains(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_skip_actions_and_offset() {
        let events = parse_skip_file(
            "0:14:08.27 --> 0:14:14\nnude image 3\n\n0:20:01 --> 0:20:02\naudio language 2\n",
            1.0,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].category, ContentCategory::Nudity);
        assert_eq!(events[1].suggested_action, EventAction::Mute);
        assert!((events[0].start - 849.27).abs() < 0.001);
    }
}
