use crate::content::{
    merge_events, stable_id, ContentCategory, ContentEvent, EventAction, Evidence,
};
use crate::media::ScanHost;
use crate::{media, probe};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use whisper_rs::{
    DtwMode, DtwModelPreset, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WordTiming {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cue {
    start: f64,
    end: f64,
    text: String,
    source: String,
    word_timings: Vec<WordTiming>,
}

/// How far down the coarseness scale to mute. Ordering matters: an entry is
/// muted when its own tier is at or above the selected one.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProfanityTier {
    /// Leave all language alone.
    Off,
    /// Only the words almost every parent would remove.
    Strong,
    /// Adds coarse-but-common words: "ass", "damn", "piss".
    #[default]
    Medium,
    /// Adds words many families only care about for a young child: "hell",
    /// "crap", "bloody", and casual blasphemy. Noisier by design.
    Mild,
}

impl ProfanityTier {
    fn allows(self, entry: ProfanityTier) -> bool {
        self != ProfanityTier::Off && entry <= self
    }

    fn severity(self) -> u8 {
        match self {
            ProfanityTier::Strong => 3,
            ProfanityTier::Medium => 2,
            _ => 1,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ProfanityTier::Strong => "Strong language",
            ProfanityTier::Medium => "Coarse language",
            _ => "Mild language",
        }
    }
}

/// Roots matched *anywhere inside* a token. Restricted to words that have no
/// innocent English containers, which is what makes containment safe: it picks
/// up "motherfucking", "clusterfuck" and "bullshitting" without enumerating
/// every inflection. "ass" and "hell" are deliberately not roots — they would
/// fire on "class", "pass", "glass", "hello" and "shell".
const PROFANITY_ROOTS: &[(&str, ProfanityTier)] = &[
    ("fuck", ProfanityTier::Strong),
    ("shit", ProfanityTier::Strong),
    ("cunt", ProfanityTier::Strong),
];

/// Matched as whole tokens only, so inflections are listed explicitly.
const PROFANITY_WORDS: &[(&str, ProfanityTier)] = &[
    ("bitch", ProfanityTier::Strong),
    ("bitches", ProfanityTier::Strong),
    ("bitching", ProfanityTier::Strong),
    ("bastard", ProfanityTier::Strong),
    ("bastards", ProfanityTier::Strong),
    ("asshole", ProfanityTier::Strong),
    ("assholes", ProfanityTier::Strong),
    ("arsehole", ProfanityTier::Strong),
    ("dick", ProfanityTier::Strong),
    ("dicks", ProfanityTier::Strong),
    ("prick", ProfanityTier::Strong),
    ("pricks", ProfanityTier::Strong),
    ("wanker", ProfanityTier::Strong),
    ("twat", ProfanityTier::Strong),
    ("whore", ProfanityTier::Strong),
    ("whores", ProfanityTier::Strong),
    ("slut", ProfanityTier::Strong),
    ("sluts", ProfanityTier::Strong),
    ("ass", ProfanityTier::Medium),
    ("asses", ProfanityTier::Medium),
    ("arse", ProfanityTier::Medium),
    ("dumbass", ProfanityTier::Medium),
    ("jackass", ProfanityTier::Medium),
    ("smartass", ProfanityTier::Medium),
    ("badass", ProfanityTier::Medium),
    ("damn", ProfanityTier::Medium),
    ("damned", ProfanityTier::Medium),
    ("damnit", ProfanityTier::Medium),
    ("dammit", ProfanityTier::Medium),
    ("goddamn", ProfanityTier::Medium),
    ("goddamned", ProfanityTier::Medium),
    ("goddammit", ProfanityTier::Medium),
    ("bugger", ProfanityTier::Medium),
    ("buggered", ProfanityTier::Medium),
    ("bollocks", ProfanityTier::Medium),
    ("piss", ProfanityTier::Medium),
    ("pissed", ProfanityTier::Medium),
    ("pissing", ProfanityTier::Medium),
    // Mild tier is opt-in because these are genuinely noisy: "hell" appears in
    // ordinary narration and "god" in "thank god" far more often than as an
    // expletive. Recall is high, precision is not.
    ("hell", ProfanityTier::Mild),
    ("crap", ProfanityTier::Mild),
    ("crappy", ProfanityTier::Mild),
    ("bloody", ProfanityTier::Mild),
    ("jesus", ProfanityTier::Mild),
    ("christ", ProfanityTier::Mild),
    ("god", ProfanityTier::Mild),
];

/// Tier of a single spoken token, or None if it is not in the lexicon. Explicit
/// whole words win over roots so a listed inflection can override.
fn profanity_of(token: &str) -> Option<(ProfanityTier, String)> {
    let clean = token
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    if clean.is_empty() {
        return None;
    }
    if let Some((_, tier)) = PROFANITY_WORDS.iter().find(|(word, _)| *word == clean) {
        return Some((*tier, clean));
    }
    PROFANITY_ROOTS
        .iter()
        .find(|(root, _)| clean.contains(root))
        .map(|(_, tier)| (*tier, clean))
}

struct Segment {
    text: String,
    /// True for SDH captions and audio description — text that describes what
    /// is on screen, rather than a character speaking.
    descriptive: bool,
}

struct TextRule {
    category: ContentCategory,
    severity: u8,
    /// P(this content actually occurs | the phrase appears in descriptive
    /// text). NOT the probability that the string matched — that was the v1
    /// mistake, which rated a bare "blood" substring at 0.88 and sorted it
    /// above everything a human had verified.
    confidence: f64,
    /// When true, a character saying this IS the content (an insult is
    /// bullying). When false, dialogue only *reports* content and is
    /// discounted — "he got shot" is weak evidence of on-screen violence.
    dialogue_is_evidence: bool,
    phrases: &'static [&'static str],
    reason: &'static str,
}

/// Below this, a match is not worth a review card — it is what stops generic
/// words from generating thousands of near-zero-value events in dialogue.
/// Sits between two deliberate anchors: a generic word spoken in dialogue
/// (0.35 x 0.6 = 0.21, suppressed) and a mild insult, where the spoken line is
/// itself the content (0.25, kept).
const MIN_EVENT_CONFIDENCE: f64 = 0.24;

const fn text_rule(
    category: ContentCategory,
    severity: u8,
    confidence: f64,
    dialogue_is_evidence: bool,
    phrases: &'static [&'static str],
    reason: &'static str,
) -> TextRule {
    TextRule {
        category,
        severity,
        confidence,
        dialogue_is_evidence,
        phrases,
        reason,
    }
}

/// Phrases are matched on whole-word boundaries, so entries here are exact
/// words or exact word sequences. Confidence is a starting estimate; run
/// `scan_report` against annotated movies and correct it from observed
/// precision rather than intuition.
const TEXT_RULES: &[TextRule] = &[
    // Frightening
    text_rule(
        ContentCategory::Frightening,
        2,
        0.70,
        false,
        &[
            "screams",
            "screaming",
            "shrieks",
            "shrieking",
            "screeches",
            "growls",
            "growling",
            "snarls",
            "snarling",
            "roars",
            "ominous music",
            "eerie music",
            "suspenseful music",
        ],
        "Frightening sound described",
    ),
    text_rule(
        ContentCategory::Frightening,
        2,
        0.35,
        false,
        &["monster", "demon", "creature", "nightmare", "haunted"],
        "Frightening subject mentioned",
    ),
    // Violence
    text_rule(
        ContentCategory::Violence,
        3,
        0.72,
        false,
        &[
            "gunshot",
            "gunshots",
            "gunfire",
            "explosion",
            "explosions",
            "bones crack",
        ],
        "Violent sound described",
    ),
    text_rule(
        ContentCategory::Violence,
        3,
        0.55,
        false,
        &[
            "stabs",
            "stabbed",
            "shoots him",
            "shoots her",
            "shot him",
            "shot her",
            "strangles",
            "covered in blood",
        ],
        "Violence or injury described",
    ),
    text_rule(
        ContentCategory::Violence,
        2,
        0.35,
        false,
        &[
            "blood", "bleeding", "wounded", "injured", "punches", "punched",
        ],
        "Violence-related description",
    ),
    // Sexual
    text_rule(
        ContentCategory::Sexual,
        3,
        0.65,
        false,
        &[
            "having sex",
            "make love",
            "making love",
            "moans",
            "moaning",
            "orgasm",
            "condom",
        ],
        "Sexual content described",
    ),
    text_rule(
        ContentCategory::Sexual,
        2,
        0.35,
        false,
        &["sexual", "seduce", "seducing", "aroused"],
        "Sexual reference",
    ),
    // Nudity
    text_rule(
        ContentCategory::Nudity,
        3,
        0.68,
        false,
        &[
            "naked",
            "nude",
            "topless",
            "undresses",
            "undressing",
            "takes off her clothes",
            "takes off his clothes",
        ],
        "Nudity described",
    ),
    // Substances
    text_rule(
        ContentCategory::Substances,
        2,
        0.62,
        false,
        &[
            "cocaine",
            "heroin",
            "overdose",
            "snorting",
            "smokes a cigarette",
            "lights a cigarette",
            "gets drunk",
        ],
        "Drug, alcohol, or smoking content described",
    ),
    text_rule(
        ContentCategory::Substances,
        1,
        0.32,
        false,
        &["drunk", "cigarette", "smoking", "drugs", "whiskey"],
        "Substance-related description",
    ),
    // Bullying — here the spoken line is the content, not a report of it.
    text_rule(
        ContentCategory::Bullying,
        2,
        0.45,
        true,
        &[
            "bully",
            "bullies",
            "bullying",
            "picks on him",
            "picks on her",
            "hate you",
            "nobody likes you",
        ],
        "Bullying described",
    ),
    text_rule(
        ContentCategory::Bullying,
        1,
        0.25,
        true,
        &["shut up", "idiot", "loser", "stupid", "freak"],
        "Cruel or insulting language",
    ),
    // Disturbing
    text_rule(
        ContentCategory::Disturbing,
        3,
        0.70,
        false,
        &[
            "suicide",
            "kill myself",
            "killing myself",
            "hang himself",
            "hang herself",
            "dead body",
            "corpse",
            "torture",
            "tortured",
        ],
        "Disturbing theme described",
    ),
    text_rule(
        ContentCategory::Disturbing,
        2,
        0.40,
        false,
        &["kidnapped", "kidnapping", "abused", "funeral"],
        "Distressing subject mentioned",
    ),
];

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TextAnalysisResult {
    pub events: Vec<ContentEvent>,
    pub source: String,
    pub cue_count: usize,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn analyze_text(
    app: tauri::AppHandle,
    path: String,
    duration: f64,
    profanity_tier: Option<ProfanityTier>,
    verify_mutes: Option<bool>,
) -> Result<TextAnalysisResult, String> {
    let tier = profanity_tier.unwrap_or_default();
    let verify = verify_mutes.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        analyze_with_host(&app, &path, duration, tier, verify)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn analyze_with_host(
    host: &dyn ScanHost,
    path: &str,
    duration: f64,
    profanity_tier: ProfanityTier,
    verify_mutes: bool,
) -> Result<TextAnalysisResult, String> {
    let scan_dir = media::cache_dir_for(host, path)?;
    let _guard = media::JobGuard::acquire(format!("text-analysis:{}", scan_dir.display()))?;
    let info = probe::probe_sync(path)?;
    let mut warnings = Vec::new();
    let mut cues = Vec::new();

    let mut subtitle_tracks: Vec<_> = info
        .tracks
        .iter()
        .filter(|t| t.kind == "subtitle" && t.is_text)
        .collect();
    subtitle_tracks.sort_by_key(|t| {
        let language_rank = match t.language.as_deref() {
            Some("eng") | Some("en") => 0,
            None => 2,
            _ => 4,
        };
        let accessibility_rank = if t.is_hearing_impaired { 0 } else { 1 };
        let default_rank = if t.is_default { 0 } else { 1 };
        (language_rank, accessibility_rank, default_rank)
    });

    if let Some(track) = subtitle_tracks.first() {
        match extract_subtitle_cues(path, track.stream_index) {
            Ok(extracted) => cues.extend(extracted),
            Err(err) => warnings.push(format!("Subtitle extraction failed: {err}")),
        }
    } else {
        warnings.push("No usable text subtitle/SDH track was found.".to_string());
    }

    let ad_track = info.tracks.iter().find(|track| {
        if track.kind != "audio" {
            return false;
        }
        track.is_visual_impaired
            || track
                .title
                .as_deref()
                .map(|title| {
                    let title = title.to_lowercase();
                    title.contains("audio description")
                        || title.contains("descriptive")
                        || title.contains("description")
                })
                .unwrap_or(false)
    });
    if let Some(track) = ad_track {
        match transcribe_track(
            host,
            path,
            track.stream_index,
            duration,
            "audio-description",
        ) {
            Ok(mut ad_cues) => cues.append(&mut ad_cues),
            Err(err) => warnings.push(format!("Audio-description track found, but {err}")),
        }
    } else if cues.is_empty() {
        let main_audio_index = probe::preferred_audio_stream(&info);
        let main_audio = info
            .tracks
            .iter()
            .find(|track| Some(track.stream_index) == main_audio_index);
        if let Some(track) = main_audio {
            match transcribe_track(host, path, track.stream_index, duration, "transcript") {
                Ok(mut transcript) => cues.append(&mut transcript),
                Err(err) => warnings.push(format!("Speech transcription unavailable: {err}")),
            }
        }
    }

    let cue_count = cues.len();
    let source = if cues.iter().any(|c| c.source == "audio-description") {
        "subtitles + audio description"
    } else if cues.iter().any(|c| c.source == "subtitle") {
        "subtitles"
    } else if cues.iter().any(|c| c.source == "transcript") {
        "speech transcript"
    } else {
        "none"
    }
    .to_string();
    let mut events: Vec<ContentEvent> = cues
        .iter()
        .flat_map(|cue| events_from_cue(cue, profanity_tier))
        .collect();

    // Confirm subtitle-derived mutes against the soundtrack before merging, so
    // a corrected range can merge with its true neighbours rather than its
    // estimated ones.
    if verify_mutes && profanity_tier != ProfanityTier::Off {
        match verify_subtitle_mutes(host, path, &info, duration, &mut events) {
            Ok((0, 0)) => {}
            Ok((confirmed, 0)) => {
                warnings.push(format!("{confirmed} mute(s) aligned to the spoken word."))
            }
            Ok((confirmed, unconfirmed)) => warnings.push(format!(
                "{confirmed} mute(s) aligned to the spoken word; {unconfirmed} could not be \
                 heard in the audio and are kept at their estimated position."
            )),
            Err(error) => warnings.push(format!(
                "Could not confirm mute timing against the audio ({error}); \
                 subtitle estimates are being used."
            )),
        }
    }
    let events = merge_events(events);
    host.emit(
        "text-analysis-progress",
        serde_json::json!({ "pct": 100.0, "events": events.len() }),
    );
    Ok(TextAnalysisResult {
        events,
        source,
        cue_count,
        warnings,
    })
}

fn extract_subtitle_cues(path: &str, stream_index: u32) -> Result<Vec<Cue>, String> {
    let ffmpeg = media::ffmpeg_path();
    let map = format!("0:{stream_index}");
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-v", "error", "-nostats", "-i", path, "-map", &map, "-f", "srt", "-",
        ],
    )?;
    let stderr_drain = media::drain_stderr(&mut child);
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("ffmpeg produced no subtitle output")?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    media::wait_checked(child, "subtitle extraction", stderr_drain)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(parse_srt(&text, "subtitle"))
}

/// Half-width of the audio window transcribed around each subtitle-derived
/// mute. Wide enough to give Whisper sentence context, narrow enough that a
/// film with fifty swear words costs about a minute rather than an hour.
const VERIFY_PAD: f64 = 6.0;
/// How far from the estimated position a spoken match may sit and still be
/// considered the same word. Subtitle *cue* timing is usually accurate to well
/// under a second; it is the position of a word inside the cue that is guessed.
const VERIFY_TOLERANCE: f64 = 3.0;
const VERIFY_CACHE_VERSION: &str = "mute-verify-v1";

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VerifiedWindow {
    start: f64,
    end: f64,
    words: Vec<WordTiming>,
}

/// Do two tokens refer to the same profanity? Handles Whisper hearing
/// "fuckin'" where the subtitle says "fucking".
fn same_profanity(a: &str, b: &str) -> bool {
    let (Some((_, left)), Some((_, right))) = (profanity_of(a), profanity_of(b)) else {
        return false;
    };
    left == right
        || PROFANITY_ROOTS
            .iter()
            .any(|(root, _)| left.contains(root) && right.contains(root))
}

/// Confirms subtitle-derived mutes against the soundtrack.
///
/// Subtitles are frequently censored ("f---"), paraphrased, or taken from a
/// different dub, and even when faithful they carry no word timings — so the
/// mute range is a guess at where in the cue the word falls. This re-transcribes
/// a few seconds around each one to find the word actually spoken and use its
/// real boundaries.
///
/// A word that cannot be confirmed is kept, not dropped: an unnecessary mute
/// costs a fraction of a second of ducked audio, a missed one is what the whole
/// tool exists to prevent. It is marked so the parent can see the difference.
///
/// Returns (confirmed, unconfirmed).
fn verify_subtitle_mutes(
    host: &dyn ScanHost,
    path: &str,
    info: &probe::VideoInfo,
    duration: f64,
    events: &mut [ContentEvent],
) -> Result<(usize, usize), String> {
    // Only subtitle cues lack word timings; transcript and audio-description
    // events already carry Whisper's own alignment.
    let targets: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event.source_key == "subtitle"
                && event.category == ContentCategory::Language
                && event.suggested_action == EventAction::Mute
        })
        .map(|(index, _)| index)
        .collect();
    if targets.is_empty() {
        return Ok((0, 0));
    }

    let stream_index =
        probe::preferred_audio_stream(info).ok_or("no audio track to confirm mutes against")?;

    // Merge the per-event windows so adjacent swear words cost one pass.
    let mut windows: Vec<VerifiedWindow> = Vec::new();
    let mut wanted: Vec<(f64, f64)> = targets
        .iter()
        .map(|&index| {
            (
                (events[index].peak_time - VERIFY_PAD).max(0.0),
                (events[index].peak_time + VERIFY_PAD).min(duration.max(1.0)),
            )
        })
        .collect();
    wanted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for span in wanted {
        match merged.last_mut() {
            Some(previous) if span.0 <= previous.1 => previous.1 = previous.1.max(span.1),
            _ => merged.push(span),
        }
    }

    let cache_path = media::cache_dir_for(host, path)?
        .join(format!("{VERIFY_CACHE_VERSION}-{stream_index}.json"));
    let mut cached: Vec<VerifiedWindow> = std::fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();

    let mut context = None;
    let total = merged.len();
    for (position, (start, end)) in merged.into_iter().enumerate() {
        if let Some(hit) = cached
            .iter()
            .find(|window| window.start <= start + 0.01 && window.end >= end - 0.01)
        {
            windows.push(hit.clone());
            continue;
        }
        if context.is_none() {
            context = Some(load_whisper(host)?);
        }
        let words = transcribe_window(
            context.as_ref().expect("just loaded"),
            path,
            stream_index,
            start,
            end,
        )?;
        let window = VerifiedWindow { start, end, words };
        cached.push(window.clone());
        windows.push(window);
        host.emit(
            "text-analysis-progress",
            serde_json::json!({
                "pct": (position + 1) as f64 / total as f64 * 100.0,
                "stage": "confirming mute timing",
            }),
        );
    }
    if cached.len() > 400 {
        let excess = cached.len() - 400;
        cached.drain(..excess);
    }
    let _ = std::fs::write(&cache_path, serde_json::to_vec(&cached).unwrap_or_default());

    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut confirmed = 0usize;
    let mut unconfirmed = 0usize;
    for &index in &targets {
        let peak = events[index].peak_time;
        let Some(word) = events[index]
            .evidence
            .first()
            .and_then(|evidence| evidence.detail.clone())
        else {
            continue;
        };
        let best = windows
            .iter()
            .enumerate()
            .filter(|(_, window)| window.start <= peak && window.end >= peak)
            .flat_map(|(window_index, window)| {
                window
                    .words
                    .iter()
                    .enumerate()
                    .map(move |(word_index, timing)| (window_index, word_index, timing))
            })
            .filter(|(window_index, word_index, timing)| {
                !used.contains(&(*window_index, *word_index))
                    && same_profanity(&timing.text, &word)
                    && ((timing.start + timing.end) / 2.0 - peak).abs() <= VERIFY_TOLERANCE
            })
            .min_by(|a, b| {
                let left = ((a.2.start + a.2.end) / 2.0 - peak).abs();
                let right = ((b.2.start + b.2.end) / 2.0 - peak).abs();
                left.partial_cmp(&right)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        match best {
            Some((window_index, word_index, timing)) => {
                used.insert((window_index, word_index));
                let event = &mut events[index];
                event.start = (timing.start - 0.12).max(0.0);
                event.end = timing.end + 0.12;
                event.peak_time = (event.start + event.end) / 2.0;
                event.confidence = 0.97;
                // The id encodes the range, so it has to be regenerated or it
                // stops matching the event it names.
                event.id = stable_id(
                    &event.source_key,
                    event.category,
                    event.start,
                    event.end,
                    &word,
                );
                event.evidence.push(Evidence {
                    source: "whisper word alignment".into(),
                    label: format!("heard \u{201c}{}\u{201d}", timing.text.trim()),
                    detail: Some("Timing confirmed against the soundtrack.".into()),
                    confidence: 0.97,
                });
                confirmed += 1;
            }
            None => {
                let event = &mut events[index];
                event.confidence = 0.60;
                event.evidence.push(Evidence {
                    source: "whisper word alignment".into(),
                    label: "not heard in the soundtrack".into(),
                    detail: Some(
                        "Subtitles can be censored, paraphrased, or from a different dub. \
                         The mute is kept at its estimated position."
                            .into(),
                    ),
                    confidence: 0.60,
                });
                unconfirmed += 1;
            }
        }
    }
    Ok((confirmed, unconfirmed))
}

fn load_whisper(host: &dyn ScanHost) -> Result<WhisperContext, String> {
    let model = host.models_dir()?.join("ggml-base.en.bin");
    if !model.exists() {
        if let Some(parent) = model.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        download_whisper_model(host, &model)?;
    }
    let mut context_parameters = WhisperContextParameters::default();
    context_parameters.dtw_parameters.mode = DtwMode::ModelPreset {
        model_preset: DtwModelPreset::BaseEn,
    };
    WhisperContext::new_with_params(&model.to_string_lossy().to_string(), context_parameters)
        .map_err(|e| format!("Could not load the local Whisper model: {e}"))
}

/// Transcribes one short span of the soundtrack, returning word timings in
/// absolute movie time. `-ss` before `-i` keeps the seek cheap on a long film;
/// ffmpeg's accurate-seek default keeps it sample-correct.
fn transcribe_window(
    context: &WhisperContext,
    path: &str,
    stream_index: u32,
    start: f64,
    end: f64,
) -> Result<Vec<WordTiming>, String> {
    let ffmpeg = media::ffmpeg_path();
    let map = format!("0:{stream_index}");
    let seek = format!("{start:.3}");
    let span = format!("{:.3}", (end - start).max(0.5));
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-v",
            "error",
            "-nostats",
            "-ss",
            &seek,
            "-i",
            path,
            "-map",
            &map,
            "-t",
            &span,
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "-",
        ],
    )?;
    let stderr = media::drain_stderr(&mut child);
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("ffmpeg produced no audio for mute confirmation")?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    media::wait_checked(child, "mute confirmation audio", stderr)?;

    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    if samples.len() < 16_000 / 2 {
        return Ok(Vec::new());
    }
    let mut state = context
        .create_state()
        .map_err(|e| format!("Could not start Whisper: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(4);
    params.set_translate(false);
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(true);
    state
        .full(params, &samples)
        .map_err(|e| format!("Whisper transcription failed: {e}"))?;
    let mut words = Vec::new();
    for segment in state.as_iter() {
        if segment.no_speech_probability() > 0.85 {
            continue;
        }
        words.extend(whisper_word_timings(&segment, start));
    }
    Ok(words)
}

fn transcribe_track(
    host: &dyn ScanHost,
    path: &str,
    stream_index: u32,
    duration: f64,
    source: &str,
) -> Result<Vec<Cue>, String> {
    let dir = media::cache_dir_for(host, path)?;
    let cached_json = dir.join(format!("{source}-v2-{stream_index}.json"));
    if let Ok(bytes) = std::fs::read(&cached_json) {
        if let Ok(cues) = serde_json::from_slice::<Vec<Cue>>(&bytes) {
            if !cues.is_empty() {
                return Ok(cues);
            }
        }
    }

    let model = host.models_dir()?.join("ggml-base.en.bin");
    if !model.exists() {
        if let Some(parent) = model.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        download_whisper_model(host, &model)?;
    }

    let model_string = model.to_string_lossy().to_string();
    let mut context_parameters = WhisperContextParameters::default();
    context_parameters.dtw_parameters.mode = DtwMode::ModelPreset {
        model_preset: DtwModelPreset::BaseEn,
    };
    let context = WhisperContext::new_with_params(&model_string, context_parameters)
        .map_err(|e| format!("Could not load the local Whisper model: {e}"))?;

    let ffmpeg = media::ffmpeg_path();
    let map = format!("0:{stream_index}");
    let mut child = media::spawn(
        &ffmpeg,
        &[
            "-v",
            "error",
            "-nostats",
            "-i",
            path,
            "-map",
            &map,
            "-vn",
            "-sn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "-",
        ],
    )?;
    let stderr = media::drain_stderr(&mut child);
    let mut reader = std::io::BufReader::with_capacity(
        16_000 * 60 * 4,
        child
            .stdout
            .take()
            .ok_or("ffmpeg produced no speech audio")?,
    );
    const CHUNK_SECONDS: usize = 300;
    const CHUNK_BYTES: usize = 16_000 * CHUNK_SECONDS * 4;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut cues = Vec::new();
    let mut chunk_index = 0usize;
    loop {
        let mut used = 0usize;
        while used < buffer.len() {
            let count = reader
                .read(&mut buffer[used..])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            used += count;
        }
        used -= used % 4;
        if used == 0 {
            break;
        }
        let samples: Vec<f32> = buffer[..used]
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect();
        let mut state = context
            .create_state()
            .map_err(|e| format!("Could not start Whisper: {e}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(4);
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(true);
        state
            .full(params, &samples)
            .map_err(|e| format!("Whisper transcription failed: {e}"))?;
        let offset = (chunk_index * CHUNK_SECONDS) as f64;
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|e| e.to_string())?
                .trim()
                .to_string();
            if text.is_empty() || segment.no_speech_probability() > 0.85 {
                continue;
            }
            cues.push(Cue {
                start: offset + segment.start_timestamp() as f64 / 100.0,
                end: offset + segment.end_timestamp() as f64 / 100.0,
                text,
                source: source.to_string(),
                word_timings: whisper_word_timings(&segment, offset),
            });
        }
        chunk_index += 1;
        let t = (chunk_index * CHUNK_SECONDS) as f64;
        let pct = if duration > 0.0 {
            (t / duration * 100.0).min(100.0)
        } else {
            0.0
        };
        host.emit(
            "text-analysis-progress",
            serde_json::json!({ "pct": pct, "t": t }),
        );
        if used < buffer.len() {
            break;
        }
    }
    media::wait_checked(child, "speech extraction", stderr)?;
    let _ = std::fs::write(cached_json, serde_json::to_vec(&cues).unwrap_or_default());
    Ok(cues)
}

fn whisper_word_timings(segment: &whisper_rs::WhisperSegment<'_>, offset: f64) -> Vec<WordTiming> {
    let mut words: Vec<WordTiming> = Vec::new();
    let segment_start = offset + segment.start_timestamp() as f64 / 100.0;
    let segment_end = offset + segment.end_timestamp() as f64 / 100.0;
    let mut token_index = 0;

    while let Some(token) = segment.get_token(token_index) {
        token_index += 1;
        let Ok(raw) = token.to_str_lossy() else {
            continue;
        };
        let raw = raw.as_ref();
        if raw.is_empty() || raw.starts_with('[') || raw.starts_with('<') {
            continue;
        }

        let data = token.token_data();
        let raw_start = if data.t_dtw >= 0 { data.t_dtw } else { data.t0 };
        if raw_start < 0 {
            continue;
        }
        let start = (offset + raw_start as f64 / 100.0).clamp(segment_start, segment_end);
        let end = (offset + data.t1.max(raw_start + 1) as f64 / 100.0)
            .clamp(start + 0.01, segment_end.max(start + 0.01));
        let begins_word = raw.chars().next().is_some_and(char::is_whitespace);
        let piece = raw.trim();
        if piece.is_empty() {
            continue;
        }

        if !begins_word {
            if let Some(word) = words.last_mut() {
                word.text.push_str(piece);
                word.end = word.end.max(end);
                continue;
            }
        }
        words.push(WordTiming {
            start,
            end,
            text: piece.to_string(),
        });
    }

    words
}

fn download_whisper_model(
    host: &dyn ScanHost,
    destination: &std::path::Path,
) -> Result<(), String> {
    let temp = destination.with_extension(format!("tmp-{}", std::process::id()));
    let mut response = reqwest::blocking::Client::builder()
        .user_agent("Videofy/0.2")
        .build()
        .map_err(|e| e.to_string())?
        .get(WHISPER_MODEL_URL)
        .send()
        .map_err(|e| format!("Whisper model download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Whisper model download failed: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let count = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        downloaded += count as u64;
        let pct = if total > 0 {
            downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        host.emit(
            "whisper-model-download",
            serde_json::json!({ "pct": pct, "downloaded": downloaded, "total": total }),
        );
    }
    std::fs::rename(temp, destination).map_err(|e| e.to_string())
}

fn parse_srt(input: &str, source: &str) -> Vec<Cue> {
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
            let (start, end) = parse_time_range(time_line)?;
            let text = lines.collect::<Vec<_>>().join(" ");
            let text = strip_markup(&text);
            if text.is_empty() {
                None
            } else {
                Some(Cue {
                    start,
                    end: end.max(start + 0.1),
                    text,
                    source: source.to_string(),
                    word_timings: Vec::new(),
                })
            }
        })
        .collect()
}

fn parse_time_range(line: &str) -> Option<(f64, f64)> {
    let mut parts = line.split("-->");
    let start = parse_timestamp(parts.next()?.trim())?;
    let end_token = parts.next()?.split_whitespace().next()?;
    let end = parse_timestamp(end_token.trim())?;
    Some((start, end))
}

fn parse_timestamp(value: &str) -> Option<f64> {
    let value = value.replace(',', ".");
    let parts: Vec<&str> = value.split(':').collect();
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

fn strip_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut inside = false;
    for ch in input.chars() {
        match ch {
            '<' | '{' => inside = true,
            '>' | '}' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits a cue into descriptive and spoken parts.
///
/// This distinction drives confidence. An SDH caption "[SCREAMS]" or an
/// audio-description line is a *description of what is happening on screen* —
/// direct evidence. A character saying "you scared me" is a *report*, and is
/// far weaker evidence that anything frightening is being shown.
fn split_segments(cue: &Cue) -> Vec<Segment> {
    if cue.source == "audio-description" {
        return vec![Segment {
            text: cue.text.to_lowercase(),
            descriptive: true,
        }];
    }
    let mut segments = Vec::new();
    let mut spoken = String::new();
    let mut caption = String::new();
    let mut depth = 0usize;
    for ch in cue.text.chars() {
        match ch {
            '[' | '(' => {
                depth += 1;
            }
            ']' | ')' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 && !caption.trim().is_empty() {
                        segments.push(Segment {
                            text: caption.trim().to_lowercase(),
                            descriptive: true,
                        });
                        caption.clear();
                    }
                }
            }
            _ if depth > 0 => caption.push(ch),
            _ => spoken.push(ch),
        }
    }
    if !caption.trim().is_empty() {
        segments.push(Segment {
            text: caption.trim().to_lowercase(),
            descriptive: true,
        });
    }
    if !spoken.trim().is_empty() {
        segments.push(Segment {
            text: spoken.trim().to_lowercase(),
            descriptive: false,
        });
    }
    segments
}

/// Lowercase word tokens. Apostrophes stay inside words so "don't" survives as
/// one token; everything else is a separator.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|token| !token.is_empty())
        .map(|token| token.trim_matches('\'').to_string())
        .filter(|token| !token.is_empty())
        .collect()
}

/// Whole-word phrase match. This is the fix for the substring rules: "blood"
/// no longer fires on "bloody hell", "drunk" no longer fires on "drunken", and
/// "ass" cannot fire from inside "class".
fn find_phrase(tokens: &[String], phrase: &str) -> Option<usize> {
    let needle: Vec<&str> = phrase.split_whitespace().collect();
    if needle.is_empty() || tokens.len() < needle.len() {
        return None;
    }
    (0..=tokens.len() - needle.len())
        .find(|&start| (0..needle.len()).all(|offset| tokens[start + offset] == needle[offset]))
}

const NEGATORS: &[&str] = &[
    "not", "no", "never", "nothing", "nobody", "didn't", "doesn't", "don't", "won't", "wouldn't",
    "isn't", "wasn't", "aren't", "can't", "couldn't", "without",
];

/// True if a negator sits within three tokens before the match, so "there was
/// no blood" and "nobody was killed" stop producing violence events.
fn is_negated(tokens: &[String], at: usize) -> bool {
    let from = at.saturating_sub(3);
    tokens[from..at]
        .iter()
        .any(|token| NEGATORS.contains(&token.as_str()))
}

fn events_from_cue(cue: &Cue, tier: ProfanityTier) -> Vec<ContentEvent> {
    let mut events = Vec::new();

    // Muting is a per-word edit, kept separate from the semantic rules below.
    // Whisper supplies real token timings; subtitles do not, so those fall back
    // to distributing words across the cue — the estimate is reflected in a
    // lower confidence rather than hidden.
    let mut push_mute =
        |start: f64, end: f64, word: &str, word_tier: ProfanityTier, exact: bool| {
            events.push(make_event(
                cue,
                (start - 0.12).max(cue.start),
                (end + 0.12).min(cue.end),
                ContentCategory::Language,
                word_tier.severity(),
                if exact { 0.98 } else { 0.85 },
                format!("{}: \u{201c}{word}\u{201d}", word_tier.label()),
                EventAction::Mute,
                word,
                Some(word.to_string()),
            ));
        };

    if !cue.word_timings.is_empty() {
        for word in &cue.word_timings {
            if let Some((word_tier, clean)) = profanity_of(&word.text) {
                if tier.allows(word_tier) {
                    push_mute(word.start, word.end, &clean, word_tier, true);
                }
            }
        }
    } else {
        let lower = cue.text.to_lowercase();
        let tokens: Vec<&str> = lower.split_whitespace().collect();
        for (index, token) in tokens.iter().enumerate() {
            let Some((word_tier, clean)) = profanity_of(token) else {
                continue;
            };
            if !tier.allows(word_tier) {
                continue;
            }
            let span = (cue.end - cue.start).max(0.2);
            let word_start = cue.start + span * index as f64 / tokens.len().max(1) as f64;
            let word_end = cue.start + span * (index + 1) as f64 / tokens.len().max(1) as f64;
            push_mute(word_start, word_end, &clean, word_tier, false);
        }
    }

    for segment in split_segments(cue) {
        let tokens = tokenize(&segment.text);
        if tokens.is_empty() {
            continue;
        }
        for rule in TEXT_RULES {
            let Some(at) = rule
                .phrases
                .iter()
                .find_map(|phrase| find_phrase(&tokens, phrase).map(|at| (at, *phrase)))
            else {
                continue;
            };
            let (index, phrase) = at;
            if is_negated(&tokens, index) {
                continue;
            }
            // Confidence answers "did this content occur?", not "did this
            // string appear?". A keyword in dialogue is weak evidence, so it
            // is discounted; nothing here suggests a decisive action.
            let confidence = if segment.descriptive || rule.dialogue_is_evidence {
                rule.confidence
            } else {
                rule.confidence * 0.6
            };
            if confidence < MIN_EVENT_CONFIDENCE {
                continue;
            }
            events.push(make_event(
                cue,
                (cue.start - 0.8).max(0.0),
                cue.end + 0.8,
                rule.category,
                rule.severity,
                (confidence * 100.0).round() / 100.0,
                rule.reason.to_string(),
                EventAction::Review,
                phrase,
                None,
            ));
        }
    }
    events
}

fn make_event(
    cue: &Cue,
    start: f64,
    end: f64,
    category: ContentCategory,
    severity: u8,
    confidence: f64,
    reason: String,
    action: EventAction,
    discriminator: &str,
    detail: Option<String>,
) -> ContentEvent {
    let source_key = cue.source.clone();
    ContentEvent {
        id: stable_id(&source_key, category, start, end, discriminator),
        start,
        end,
        peak_time: (start + end) / 2.0,
        category,
        severity,
        confidence,
        reason,
        suggested_action: action,
        evidence: vec![Evidence {
            source: cue.source.clone(),
            label: cue.text.clone(),
            detail,
            confidence,
        }],
        source_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt_and_generates_precise_language_event() {
        let cues = parse_srt(
            "1\n00:00:10,000 --> 00:00:12,000\nWhat the fucking hell?\n",
            "subtitle",
        );
        assert_eq!(cues.len(), 1);
        let events = events_from_cue(&cues[0], ProfanityTier::Medium);
        let language = events
            .iter()
            .find(|e| e.category == ContentCategory::Language)
            .unwrap();
        assert_eq!(language.suggested_action, EventAction::Mute);
        assert!(language.start >= 10.0 && language.end <= 12.0);
    }

    fn cue_of(text: &str, source: &str) -> Cue {
        Cue {
            start: 10.0,
            end: 12.0,
            text: text.to_string(),
            source: source.to_string(),
            word_timings: Vec::new(),
        }
    }

    fn categories(text: &str, source: &str) -> Vec<ContentCategory> {
        events_from_cue(&cue_of(text, source), ProfanityTier::Medium)
            .iter()
            .map(|event| event.category)
            .collect()
    }

    fn muted_words(text: &str, tier: ProfanityTier) -> Vec<String> {
        events_from_cue(&cue_of(text, "subtitle"), tier)
            .into_iter()
            .filter(|event| event.suggested_action == EventAction::Mute)
            .map(|event| event.reason)
            .collect()
    }

    #[test]
    fn matches_whisper_spelling_variants_of_the_same_word() {
        // Whisper routinely writes "fuckin'" where the subtitle says "fucking";
        // the confirmation step has to treat those as the same word.
        assert!(same_profanity("fucking", "fuckin"));
        assert!(same_profanity("Fucking,", "fucked"));
        assert!(same_profanity("bullshit", "shit"));
        assert!(same_profanity("damn", "damn"));
    }

    #[test]
    fn does_not_match_different_words_or_clean_speech() {
        assert!(!same_profanity("fucking", "shit"));
        assert!(!same_profanity("damn", "hell"));
        assert!(!same_profanity("hello", "hell"));
        assert!(!same_profanity("class", "ass"));
        assert!(!same_profanity("nothing", "anything"));
    }

    #[test]
    fn roots_catch_inflections_and_compounds() {
        // The v1 list matched whole tokens only, so every one of these escaped.
        for text in [
            "That is fucking ridiculous",
            "He fucks it up",
            "What a shitty day",
            "Total bullshit",
            "You motherfucking idiot",
            "A clusterfuck",
        ] {
            assert!(
                !muted_words(text, ProfanityTier::Strong).is_empty(),
                "{text:?} should produce a mute"
            );
        }
    }

    #[test]
    fn containment_roots_do_not_fire_on_innocent_words() {
        // Why "ass" and "hell" are whole-word entries and not roots.
        for text in [
            "The whole class went to the glass house",
            "Please pass the grass seed",
            "Hello, ring the shell bell",
            "She will assist the assembly",
        ] {
            assert!(
                muted_words(text, ProfanityTier::Mild).is_empty(),
                "{text:?} must not be muted"
            );
        }
    }

    #[test]
    fn tiers_gate_which_words_are_muted() {
        let line = "Oh hell, that damn fucking door";
        assert_eq!(muted_words(line, ProfanityTier::Off).len(), 0);
        assert_eq!(muted_words(line, ProfanityTier::Strong).len(), 1); // fucking
        assert_eq!(muted_words(line, ProfanityTier::Medium).len(), 2); // + damn
        assert_eq!(muted_words(line, ProfanityTier::Mild).len(), 3); // + hell
    }

    #[test]
    fn severity_and_label_follow_the_tier() {
        let events = events_from_cue(&cue_of("damn it", "subtitle"), ProfanityTier::Mild);
        let mute = events
            .iter()
            .find(|event| event.suggested_action == EventAction::Mute)
            .expect("damn should mute at mild");
        assert_eq!(mute.severity, 2, "medium-tier word keeps medium severity");
        assert!(mute.reason.starts_with("Coarse language"));
    }

    #[test]
    fn estimated_subtitle_timing_is_less_confident_than_whisper_timing() {
        let estimated =
            events_from_cue(&cue_of("what the fuck", "subtitle"), ProfanityTier::Strong);
        let mut timed = cue_of("what the fuck", "subtitle");
        timed.word_timings = vec![WordTiming {
            start: 11.0,
            end: 11.4,
            text: "fuck".into(),
        }];
        let exact = events_from_cue(&timed, ProfanityTier::Strong);
        assert!(
            exact[0].confidence > estimated[0].confidence,
            "word-aligned timing should outrank a guess at where in the cue the word falls"
        );
    }

    #[test]
    fn word_boundaries_stop_the_substring_false_positives() {
        // Every one of these fired a high-confidence rule in v1.
        assert!(!categories("Oh bloody hell, we are late.", "subtitle")
            .contains(&ContentCategory::Violence));
        assert!(!categories("A drunken sailor sang.", "subtitle")
            .contains(&ContentCategory::Substances));
        assert!(!categories("She looked smoking hot.", "subtitle")
            .contains(&ContentCategory::Substances));
    }

    #[test]
    fn negation_suppresses_the_match() {
        assert!(!categories("There was no blood at all.", "subtitle")
            .contains(&ContentCategory::Violence));
        // Same phrase without the negator, in text that describes the screen.
        assert!(
            categories("Blood pooled on the floor.", "audio-description")
                .contains(&ContentCategory::Violence)
        );
        assert!(
            !categories("No blood pooled on the floor.", "audio-description")
                .contains(&ContentCategory::Violence)
        );
    }

    #[test]
    fn captions_are_stronger_evidence_than_dialogue() {
        let caption = events_from_cue(&cue_of("[GUNSHOTS]", "subtitle"), ProfanityTier::Medium);
        let dialogue = events_from_cue(
            &cue_of("I heard gunshots last night", "subtitle"),
            ProfanityTier::Medium,
        );
        let caption_confidence = caption[0].confidence;
        let dialogue_confidence = dialogue[0].confidence;
        assert!(
            caption_confidence > dialogue_confidence,
            "caption {caption_confidence} should outrank dialogue {dialogue_confidence}"
        );
    }

    #[test]
    fn keyword_rules_no_longer_suggest_destructive_actions() {
        for event in events_from_cue(&cue_of("[GUNSHOTS]", "subtitle"), ProfanityTier::Medium) {
            assert_eq!(event.suggested_action, EventAction::Review);
            assert!(
                event.confidence <= 0.75,
                "a keyword match must not claim near-certainty"
            );
        }
    }

    #[test]
    fn generic_words_in_dialogue_fall_below_the_floor() {
        // "blood" spoken in dialogue is 0.35 * 0.6 = 0.21, under the floor.
        assert!(categories("Blood is thicker than water.", "subtitle").is_empty());
        // The same word in audio description is a real observation.
        assert!(
            categories("Blood drips from the wall.", "audio-description")
                .contains(&ContentCategory::Violence)
        );
    }

    #[test]
    fn parses_webvtt_style_time() {
        assert_eq!(parse_timestamp("01:02:03.500"), Some(3723.5));
        assert_eq!(parse_timestamp("02:03.250"), Some(123.25));
    }

    #[test]
    fn prefers_whisper_word_timing_for_profanity() {
        let cue = Cue {
            start: 10.0,
            end: 20.0,
            text: "That is fucking ridiculous".into(),
            source: "transcript".into(),
            word_timings: vec![WordTiming {
                start: 13.4,
                end: 13.9,
                text: "fucking".into(),
            }],
        };
        let event = events_from_cue(&cue, ProfanityTier::Medium)
            .into_iter()
            .find(|event| event.category == ContentCategory::Language)
            .unwrap();
        assert!((event.start - 13.28).abs() < 0.001);
        assert!((event.end - 14.02).abs() < 0.001);
    }
}
