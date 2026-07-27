# Videofy Detection Overhaul — Implementation Summary

Date: 2026-07-26

## Outcome

Videofy has been changed from a loudness-based jump-scare detector into a
multi-source content-review tool intended to help a parent prepare a cleaner
version of a movie for a six-year-old.

Loudness is still used, but only as a weak clue. It no longer represents the
app's opinion of whether a scene is unsuitable.

The implemented system now combines:

- Embedded subtitles and SDH captions
- Audio-description tracks
- Local Whisper speech transcription when timed text is unavailable
- YAMNet semantic sound-event classification
- Scene-change frame sampling with conservative built-in visual heuristics
- Does the Dog Die? timestamped guide data
- Imported SRT, VTT, and SKP timing files
- The original sudden-loudness detector as supporting evidence

Results from different sources can be fused into one review event when they
refer to the same category and moment. The parent makes the final decision:
**Cut**, **Mute**, or **Keep**.

## Important product decision

The Ollama/local-LLM route has been removed completely.

There is:

- No Ollama setting in the interface
- No `visionModel` application setting
- No request to `127.0.0.1:11434`
- No image-to-base64 Ollama payload
- No Ollama request or response types in the Rust backend
- No direct `base64` dependency for that integration

The Picture pass remains a deliberately low-confidence candidate generator. It
does not claim to understand the semantic meaning of a frame.

## 1. Unified content-event model

Added `src-tauri/src/content.rs`.

The previous numeric scare candidate has been replaced with a stable
`ContentEvent` containing:

- Stable string ID
- Start, end, and peak timestamps
- Content category
- Severity from 1 to 3
- Detection confidence from 0 to 1
- Short factual reason
- Suggested action: review, cut, or mute
- Evidence records with source, label, detail, and confidence
- Source key for merging, caching, and diagnostics

Current categories:

- `frightening`
- `violence`
- `sexual`
- `nudity`
- `language`
- `substances`
- `bullying`
- `disturbing`

Severity and confidence are intentionally separate. A severe event can still
be uncertain, and a mild event can be detected confidently.

The frontend keeps raw source events separately from fused display events.
Corroborating events in the same category and time range are combined, their
evidence is preserved, and confidence receives a small corroboration increase.
Conflicting actions such as an exact profanity mute and a larger guide cut are
kept separate.

## 2. Track discovery, subtitles, SDH, and audio description

Expanded `src-tauri/src/probe.rs`.

FFprobe results now expose:

- Absolute stream index
- Stream type and codec
- Language
- Title
- Default and forced dispositions
- Hearing-impaired/SDH disposition
- Visual-impaired/audio-description disposition
- Whether a subtitle stream is text-based and extractable

A shared main-audio selection rule now prefers:

1. The default non-audio-description track
2. Another non-audio-description audio track
3. Any available audio track as a final fallback

That selection is used by:

- Proxy generation
- Loudness analysis
- Waveform generation
- YAMNet sound analysis
- Main-dialogue transcription

`src-tauri/src/text_analysis.rs` selects a suitable English text subtitle,
preferring SDH and default tracks. It also detects audio-description tracks
from dispositions and common track titles.

Timed text is scanned for:

- Strong language
- Frightening descriptions and sound captions
- Violence, injury, and death
- Sexual content and references
- Nudity
- Smoking, alcohol, and drugs
- Bullying and cruel language
- Other disturbing themes

## 3. Local Whisper fallback and profanity muting

When no usable timed subtitle is available, the app can transcribe the selected
main English soundtrack locally with Whisper `base.en`.

If an audio-description track exists, that track is transcribed as an
additional source even when ordinary subtitles are present.

Implementation details:

- Uses `whisper-rs` with Metal support on macOS
- Downloads `ggml-base.en.bin` only when transcription is required
- Streams 16 kHz mono floating-point audio from FFmpeg
- Processes five-minute chunks to keep memory bounded
- Emits model-download and transcription progress
- Caches transcript cues as JSON
- Preserves Whisper token/word timestamps in the cache

Profanity results use short **Mute** ranges. Whisper-generated profanity uses
the model's word timing with a small safety pad. Ordinary subtitle cues usually
do not include word timestamps, so those use a conservative estimate of the
word's position within the subtitle cue.

## 4. External guides and timing-file import

Added `src-tauri/src/guides.rs`.

### Does the Dog Die?

The app can:

- Search by inferred or edited movie title and year
- Retrieve the selected title's topic statistics
- Fetch substantive timestamped ratings in one ratings request
- Read trigger, cue, trigger-time, and safe-to-resume fields
- Distinguish professional Scene Alerts from community ratings
- Map relevant topics into Videofy's content categories
- Explain when the API account can see title warnings but not timestamped
  ratings

The API key is entered in the guide tools and stored in the Tauri webview's
local storage. Movie media is not sent to the guide service.

### Imported timing files

The app accepts:

- SRT
- WebVTT
- SKP/VideoSkip-style ranges

Imported labels are mapped to content categories. SKP entries can suggest
audio muting or video cutting.

Where's the Jump-style warning subtitles are treated specially: the cue end is
used as the likely scare point because those subtitle files generally warn
shortly before the impact.

### Edition synchronisation

The user can always enter a manual timing offset.

When the manual offset is zero and enough frightening timestamps exist, the
frontend attempts a conservative automatic offset:

- Imported frightening events are matched to local loudness-impact anchors
- Differences beyond 30 seconds are rejected
- At least three closely agreeing matches are required
- A single mean offset is applied only to the inliers
- The applied offset is recorded as evidence

This is a single-offset synchroniser, not a full time-warp solution.

## 5. Semantic sound-event classification

Added `src-tauri/src/audio_events.rs`.

The app uses the YAMNet ONNX model and AudioSet labels to identify semantic
sound clues such as:

- Screaming, shrieking, wailing, crying, and growling
- Gunshots and gunfire
- Explosions
- Breaking glass and smashing
- Crashes, slaps, thumps, and thuds
- Moaning and groaning, at a higher threshold

Implementation details:

- YAMNet and its labels are downloaded on first use
- ONNX Runtime performs inference locally
- FFmpeg streams the preferred main soundtrack as 16 kHz mono audio
- Audio is processed in eight-second chunks
- Model progress is emitted to the UI
- Results are cached by media file, model version, and selected audio stream
- Nearby same-source results are merged before reaching the frontend

YAMNet findings default to **Review**. A sound label alone is not treated as
proof that the scene is unsuitable.

## 6. Scene-based visual candidate scan

Added `src-tauri/src/scene_analysis.rs`.

The Picture pass:

- Uses FFmpeg scene-change detection
- Always includes the first frame
- Extracts 320-pixel-wide JPEG review frames
- Uses FFmpeg `showinfo` timestamps rather than image filename PTS assumptions
- Samples pixel statistics from each selected frame
- Emits progress to the UI
- Removes temporary frames after analysis
- Caches final visual events

The built-in heuristics create low-confidence review clues for:

- Strong red regions that may indicate blood, fire, or violent imagery
- Large skin-like regions that may indicate nudity
- Very dark scenes with red contrast that may be frightening

These checks will produce false positives. They are intentionally labeled as
possible content and default to **Review**.

## Review interface

`src/components/SegmentsPanel.tsx` has been rebuilt around content events.

It now includes:

- Text, Sound, Picture, and Guide coverage indicators
- Progressive scan progress and error reporting
- Category filters
- Minimum severity filter
- Time, severity, and confidence sorting
- Evidence expansion
- Per-event Cut, Mute, and Keep controls
- Bulk Cut, Mute, and Keep
- Does the Dog Die? title/year/API-key controls
- SRT/VTT/SKP import
- Manual guide offset
- Semantic rescan control

The timeline now draws:

- Pending content clues
- Approved cuts
- Approved mutes
- Kept/ignored clues
- Manual cuts

Keyboard navigation and event IDs have been migrated to the new string-based
event model.

## Decision persistence

Per-movie project state is saved in local storage using a key derived from:

- Source path
- Source size
- Source duration

Saved state includes:

- Event decisions
- Manual cuts
- Next manual-cut ID
- Imported/user guide events

Stable detector IDs allow cached scans to recover previous decisions. When
additional evidence changes a fused event ID during a progressive scan,
decisions are carried to the new event when category, source, and time overlap.

## Export changes

`src-tauri/src/export.rs` now accepts both cut and mute ranges.

Cut behavior remains conservative:

- Overlapping cuts are merged
- Kept sections begin at safe keyframes
- Video frames from a cut are not allowed to leak back into the output

Mute behavior:

- Source mute ranges are remapped onto the post-cut output timeline
- Mutes that overlap removed sections are clipped or discarded
- Overlapping output mutes are merged
- Video and compatible subtitles remain stream-copied
- Audio is re-encoded to AAC only when at least one mute exists
- Cuts-only exports continue to use stream copy

The export result now reports both removed duration and muted duration.

## Cache corrections

Old cache names were versioned where audio-track selection changed.

The following caches now include the preferred stream or a new version:

- Loudness series
- Stereo waveform
- Proxy
- YAMNet result
- Whisper transcript with word timing

This prevents an old first-audio-track cache from being silently reused after
the main-track selection logic changed.

Duplicate semantic jobs for the same movie are guarded in Rust so repeated
scan requests cannot race on the same model or cache output.

## Main files added

- `src-tauri/src/content.rs`
- `src-tauri/src/text_analysis.rs`
- `src-tauri/src/guides.rs`
- `src-tauri/src/audio_events.rs`
- `src-tauri/src/scene_analysis.rs`
- `DETECTION_IMPLEMENTATION_SUMMARY.md`

## Main files substantially changed

- `src-tauri/src/probe.rs`
- `src-tauri/src/analysis.rs`
- `src-tauri/src/proxy.rs`
- `src-tauri/src/waveform.rs`
- `src-tauri/src/export.rs`
- `src-tauri/src/lib.rs`
- `src/store.ts`
- `src/types.ts`
- `src/components/SegmentsPanel.tsx`
- `src/components/Timeline.tsx`
- `src/components/Editor.tsx`
- `src/hooks/useShortcuts.ts`
- `README.md`

## Validation completed

Rust:

```text
cargo test
14 passed; 0 failed
```

The tests cover:

- Stable content-event IDs
- Loudness parsing and behavior
- Semantic sound-label mapping
- SRT and WebVTT timing parsing
- Profanity mute timing
- Whisper word-timing preference
- Guide/SKP parsing and offsets
- Scene timestamp parsing
- Cut planning
- Mute remapping after cuts

Frontend:

```text
npm run build
TypeScript compilation passed
Vite production build passed
```

Formatting and whitespace validation also pass.

## Known limitations

This app remains a review accelerator, not a guarantee that a movie is safe for
a child.

Important limitations:

- Whisper is currently English-only
- Keyword rules cannot understand every context
- YAMNet identifies sounds, not the meaning of the surrounding scene
- The visual scan is heuristic and low-confidence
- Subtitles do not describe all visual content
- Audio-description tracks are not available in every file
- Does the Dog Die? exact timestamps depend on API tier and title coverage
- Imported timestamps can drift between Blu-ray, streaming, PAL, extended, and
  theatrical editions
- Automatic guide sync currently estimates only one constant offset
- Model thresholds and padding still need calibration against real annotated
  movies

Every flagged event should still be reviewed before export.
