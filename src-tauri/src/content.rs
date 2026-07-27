use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ContentCategory {
    Frightening,
    Violence,
    Sexual,
    Nudity,
    Language,
    Substances,
    Bullying,
    Disturbing,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EventAction {
    Review,
    Cut,
    Mute,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub source: String,
    pub label: String,
    pub detail: Option<String>,
    pub confidence: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContentEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub peak_time: f64,
    pub category: ContentCategory,
    /// How unsuitable the content may be: 1=mild, 2=moderate, 3=strong.
    pub severity: u8,
    /// How confident the detector is that the described content occurs.
    pub confidence: f64,
    pub reason: String,
    pub suggested_action: EventAction,
    pub evidence: Vec<Evidence>,
    pub source_key: String,
}

pub fn stable_id(
    source: &str,
    category: ContentCategory,
    start: f64,
    end: f64,
    discriminator: &str,
) -> String {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    category.hash(&mut hasher);
    ((start * 10.0).round() as i64).hash(&mut hasher);
    ((end * 10.0).round() as i64).hash(&mut hasher);
    discriminator.hash(&mut hasher);
    format!("evt-{:016x}", hasher.finish())
}

pub fn merge_events(mut events: Vec<ContentEvent>) -> Vec<ContentEvent> {
    events.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.category).cmp(&format!("{:?}", b.category)))
    });
    let mut merged: Vec<ContentEvent> = Vec::new();
    for event in events {
        let mergeable = merged.last().is_some_and(|prev| {
            prev.category == event.category
                && event.start <= prev.end + 1.0
                && prev.source_key == event.source_key
        });
        if mergeable {
            let prev = merged.last_mut().expect("checked above");
            prev.end = prev.end.max(event.end);
            if event.confidence > prev.confidence {
                prev.peak_time = event.peak_time;
                prev.reason = event.reason.clone();
            }
            prev.confidence = prev.confidence.max(event.confidence);
            prev.severity = prev.severity.max(event.severity);
            prev.evidence.extend(event.evidence);
            prev.id = stable_id(
                &prev.source_key,
                prev.category,
                prev.start,
                prev.end,
                &prev.reason,
            );
        } else {
            merged.push(event);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_repeatable() {
        let a = stable_id("subtitle", ContentCategory::Language, 1.23, 2.34, "word");
        let b = stable_id("subtitle", ContentCategory::Language, 1.24, 2.31, "word");
        assert_eq!(a, b);
    }
}
