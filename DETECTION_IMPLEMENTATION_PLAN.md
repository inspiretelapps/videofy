# Videofy Detection — Implementation Plan

Written: 2026-07-26
Supersedes the "recommended continuation order" in `DETECTION_REVIEW_HANDOFF.md` §12.
Companion to that document; it remains the record of *why* the architecture looks like this.

## 0. Decision and its consequences

**Decision: no local LLM.** The Ollama vision-verifier path is cut. A 4B-class VLM would need
~3.5 GB resident alongside FFmpeg, Whisper, ONNX and video playback on a 16 GB M4 — and at
2–4 s per frame it would add 15–50 minutes to every scan.

Three consequences follow, and the rest of this plan is shaped by them:

1. **The realistic ceiling drops, and the product framing must move with it.** Without semantic
   models the tool will not reliably catch quiet menace, sexual tension, emotional cruelty, or
   context-dependent threat ("I'll kill you" as a joke vs. a threat). Videofy is a **fast
   candidate generator with a traceable evidence trail**, not a classifier that says what is
   unsafe. Every UI string should reflect that.

2. **External human-curated data is promoted from supporting signal to primary semantic source.**
   Does the Dog Die?, Where's the Jump, and user-supplied timing files are now the only components
   in the system that actually understand *meaning*. `guides.rs` moves up the priority list.

3. **Review throughput matters more than it did.** Precision will be lower than an LLM-backed
   system would give, so the parent will sift more candidates. Time-per-candidate becomes the
   metric to optimise, which makes fusion and keyboard-driven review high-value work rather than
   polish.

## 1. Verified state (checked, not assumed)

- `cargo check` clean; **13/13 Rust tests pass**, including the export mute-remap test.
  One dead-code warning: `media::find_optional_tool`.
- `src/store.ts` exists and is rewritten against the new contracts — handoff §9's
  "store.ts is deleted" is stale. Per-source scan state, category filters, guide settings,
  `deriveEdits` and localStorage persistence are all present.
- Frontend does not build: ~20 TS errors confined to `Editor.tsx`, `Timeline.tsx`,
  `hooks/useShortcuts.ts`, plus `SegmentsPanel.tsx` deleted with no replacement.
- **Scene timestamps are broken** (below). Every visual event lands at t≈0.
- Ollama was never installed, so `verify_with_ollama` has never executed on this machine.
- `-af volume=0:enable=...` **does** apply to every mapped audio track — verified on a
  two-track fixture (−91 dB in-window, −17.7 dB outside). Handoff §11's multi-track mute
  risk is closed.

---

## Phase 0 — Unbreak (do first, ~2 days)

**0.1 Fix scene-frame timestamps.** `-frame_pts 1` writes the PTS *after* the image2 muxer
rescales into the output stream timebase (1/25), not the `settb=AVTB` filter timebase. Verified
on a fixture with a scene change at 12.0 s: `showinfo` reports `pts:12000000`, the file written is
`frame-...300.jpg` (12 × 25), and `scene_analysis.rs:250` computes `300 / 1_000_000` = 0.0003 s.

Fix: add `-enc_time_base 1/1000000` to the arg list in `scene_analysis.rs:109-131`. Verified to
produce `frame-0000000012000000.jpg` → 12.0 s. Add a regression test on a generated fixture with
known scene-change times.

**0.2 Delete the Ollama path.** Remove `verify_with_ollama`, `OllamaRequest`, `OllamaMessage`,
`OllamaResponse`, `VisionDecision`, `parse_category` and its test from `scene_analysis.rs`; drop
the `base64` dependency (used nowhere else); remove `visionModel` from `Settings` in `store.ts`
and the `visionModel` argument from the `analyze_scenes` command. ~80 lines, recoverable from git
if the decision is ever revisited. Also re-enable caching unconditionally — `scene_analysis.rs:86`
currently skips the cache whenever a model is configured, which becomes dead logic anyway.

**0.3 Restore the frontend build.** Migrate `Editor.tsx`, `Timeline.tsx` and `useShortcuts.ts`
from numeric candidate IDs to string event IDs; replace `SegmentsPanel.tsx` with an event-list
panel. Target: `npx tsc --noEmit` clean. Keep the panel deliberately minimal — Phase 2 changes
what it renders.

**0.4 Serialize the scan pipeline.** `store.ts:334-368` fires text + audio + vision concurrently
on top of the proxy, keyframe, loudness and waveform passes already running from `openFile` —
six concurrent full-file decodes, one of them Whisper. Introduce a single job queue in Rust with
a bounded worker count. Order: probe → proxy + keyframes (fast, needed for playback) → waveform →
loudness → subtitles → YAMNet → scenes → Whisper (slowest, last).

**0.5 YAMNet: use the selected audio stream.** `audio_events.rs:83-84` hardcodes `0:a:0`. Use the
default/main track the new probe exposes, same selection logic as `text_analysis.rs:99-103`.

---

## Phase 1 — Measure before tuning (~2 days, unblocks everything else)

Every threshold in the codebase is a guess: 0.18, 0.20, 0.28, 0.065, 0.34, 0.43, 0.86, 0.095, plus
all the padding constants. There are currently six detectors and zero measurements.

**1.1 Add a `scan_report` dev command.** Runs every detector against a path and writes JSON:
events per source, per category, per hour; duration of each pass; total wall-clock. No UI needed.
This is the instrument; build it before tuning anything.

**1.2 Annotate three movies you already know.** Unsafe ranges + category, by hand. Two hours of
work. This is the entire basis for every threshold decision that follows, and handoff §14's
benchmark in miniature.

**1.3 Publish the baseline.** Candidates/hour per source against those three films. Expect YAMNet
and the text rules to be one to two orders of magnitude too noisy. Record it — it is the number
Phase 2 and Phase 3 are trying to move.

---

## Phase 2 — Fusion: make the list reviewable (highest product value)

Today a single jump scare produces four independent cards — loudness, YAMNet, a `[SCREAMS]`
caption, and a dark-frame clue — because `content.rs:78-82` merges only within the same category
*and* the same `source_key`, and the store concatenates. Each detector added multiplies the list.
This is the work that decides whether the tool saves time.

**2.1 Region model.** Group events across all sources within N seconds (start N=4, tune from
Phase 1) into one `ContentRegion` carrying stacked evidence. One review card per region. Expect
this alone to shrink the list by roughly an order of magnitude.

**2.2 Cross-source agreement drives confidence.** Independent sources agreeing should raise
confidence; a lone weak clue should lower it. A dark frame *plus* a scream caption *plus* a
loudness jump is a real jump scare; a dark frame alone is nothing. This is the one place the
system can be genuinely smarter than its parts without a semantic model.

**2.3 Decouple IDs from tuning.** `content.rs:53-67` hashes the *padded* start/end, so changing
any padding constant re-IDs every event and silently detaches every saved decision in
`eventStatus`. Handoff §14.5 wants those decisions kept as calibration data — as built, the two
goals are mutually exclusive. Key persistence on `(category, peak-time bucket)` or store decisions
as time ranges resolved against events at load. Separately, replace `DefaultHasher` — it is
documented as unstable across Rust releases, so a toolchain bump has the same effect.

**2.4 Precision budget.** Cap surfaced regions per hour (start ~40) and show what was suppressed
and why. Handoff §14.4 weights recall over precision, which is correct for safety but in tension
with the product goal: a list of thousands gets ignored wholesale, which is a *worse* safety
outcome than a shorter, better list. A visible cap with a "show suppressed" escape hatch resolves it.

---

## Phase 3 — Accuracy without semantic models

**3.1 Rewrite the text rules properly.** `text_analysis.rs:480-590` uses plain substring matching,
so "blood" fires on "bloody hell", "drunk" on "drunken", "smoking" on "smoking hot", and
"idiot"/"loser"/"shut up" fire constantly in exactly the children's films being screened — each at
0.72–0.9 confidence, which sorts them to the top. Required changes:

- Word-boundary matching, not `contains`.
- Multi-word phrases as phrases; negation and hedge guards ("not", "never", "would have").
- **Confidence must mean P(content occurred | evidence).** For a bare keyword match that is
  ~0.3–0.5, never 0.88. Calibrate against the Phase 1 annotations.
- Separate the lexicon from the code into a data file so tuning does not require a rebuild.

**3.2 Consider a sentence-embedding pass (open decision).** A MiniLM-class ONNX text encoder is
~90 MB and runs in milliseconds per cue via the `ort` runtime already in the build. Scoring cue
embeddings against concept prototypes catches "he's going to hurt you" without "hurt" being in any
list. This is **not** an LLM — no generative model, no multi-GB residency, negligible RAM. It is
the largest non-LLM recall gain available on the text side. Your call; I recommend it.

**3.3 Recalibrate YAMNet.** `audio_events.rs:266-298` fires at 0.18/0.20 on labels including
"crash", "smash", "thump", "thud", "slap" and "breaking" — routine film foley. Drop those labels
or raise them to ~0.5, set the rest from Phase 1 data. Also add chunk overlap: 8 s chunks at a
0.48 s hop leave the tail of each chunk without a full patch, a systematic blind spot every 8 s.

**3.4 Decide the vision path (open decision).** With the VLM gone, `scene_analysis.rs` is colour
fractions, and `:167-169` gates on them, so recall is capped by the weakest component: strangling,
drowning, a gun aimed at a child, a monster in a lit room, and dim-lit nudity are all invisible.
Two honest options:

- **Accept it as a weak clue generator.** Keep confidences at 0.36–0.48, never let it raise a
  region on its own, only let it corroborate. Cheap, honest, low value.
- **Add a CLIP/SigLIP zero-shot gate.** ~150 MB ONNX, ~10 ms/frame, no generative model, uses the
  `ort` runtime already present for YAMNet — resource profile closer to YAMNet than to an LLM.
  Scoring frames against concept prompts ("a bloody wound", "a person aiming a gun", "a nude
  figure", "a monster's face") gives real semantic gating for near-zero compute and uncaps the
  visual path.

I recommend the CLIP gate and want to be explicit that it is not the thing you rejected — but it
is genuinely optional, and Phases 0–2 are worth more than either option here.

**3.5 Elevate the guide integration.** Per §0, this is now the primary semantic signal. Confirm
the DDD API response shape against a real key, add title/year confirmation UI, and make imported
guide events visually distinct — they are human-verified and deserve to outrank every detector.
Keep the global offset for now; note that alternate editions may need piecewise alignment.

**3.6 Loudness stays corroboration-only.** It should never raise a region alone. This is already
close to true given its confidence values; make it structural in the fusion rules.

---

## Phase 4 — Muting (the best-designed feature; three fixes)

**4.1 Real word timing.** Subtitle cues carry no word timestamps, so `text_analysis.rs:460-478`
distributes words evenly across the cue — a 3 s cue with 12 words gives each 250 ms ±120 ms pad.
Speech is not uniform, and subtitles are frequently paraphrased or censored relative to the audio,
so the word may not be at that cue at all. Use **subtitles to decide *what*, Whisper token-DTW to
decide *when***: `text_analysis.rs:189-191` already enables `DtwMode::ModelPreset` and
`set_token_timestamps(true)`, then reads only segment-level times. Run Whisper on ±6 s windows
around each flagged profanity — 10–30 windows per film, seconds of compute, true word boundaries.

**4.2 Make it sound right.** `volume=0:enable='between(...)'` is a hard gate: a click at each edge
and a conspicuous hole. Duck to ~−30 dB with a 10–15 ms ramp instead. For a six-year-old, a dip
nobody registers beats a silence that prompts "what did he say?".

**4.3 Stop degrading the audio.** Any mute re-encodes *all* audio to AAC 192k for the whole film —
on a 5.1 track that is 192k across six channels, and it discards the lossless property of the
entire movie to mute a handful of words. Scale bitrate by channel count, or offer FLAC/ALAC into
MKV.

**4.4 Structure the lexicon.** The current list is exact-match on cleaned tokens, so it misses
"fucks", "shitty", "asses", and omits words that matter more for a six-year-old than some included
("damn", "hell", "crap", "ass", "god"). Tier it — strong / medium / mild-for-a-six-year-old — and
let the parent pick the tier.

**4.5 Fix double-counted dialogue.** When an audio-description track *and* subtitles both exist,
`text_analysis.rs:93-110` keeps both; AD-track audio contains all the dialogue, so every line
appears twice under different `source` values and `merge_events` will not merge across source keys.
Phase 2's region model largely absorbs this, but dedupe at the cue level too.

---

## Phase 5 — Review UX (worth more now than before the LLM decision)

**5.1 Keyboard-first review loop.** Next/prev region, cut/mute/keep, undo, jump-to-peak, without
leaving the keyboard. The metric is seconds-per-candidate; this is where it is won.

**5.2 Region cards** per handoff §13, but one card per *region* with evidence grouped by source
and an expandable transcript/caption detail. Batch actions over filtered sets.

**5.3 Coverage panel.** Explicitly state what was and was not scanned — subtitles found/absent,
Whisper used/skipped, guide matched/unconfigured. Handoff §13's closing point is right and matters
more now: zero detections must never read as "safe".

**5.4 Move persistence off localStorage.** Write a sidecar JSON next to the movie, keyed by a
content hash rather than `info.path` (`store.ts:743-745`), so decisions survive moving or renaming
the file — and so `userEvents` are not held in a ~5 MB browser quota.

---

## Phase 6 — Robustness (before it is used on real movies unattended)

- Cancellation for the blocking model work; job guards against duplicate scans.
- Checksum the model downloads; clean stale `.tmp-<pid>` files.
- Export matrix test: no audio track, multiple audio tracks, MKV/MP4/MOV/M4V, PGS subtitles into
  MP4 (will fail — decide whether to drop subtitles or refuse the container).
- Integration test that mute alignment survives concat + keyframe snapping in real output media,
  not just the unit-level remap.
- Model licence and attribution text in the app and README (Whisper, YAMNet, AudioSet class map).
- Remove or use `media::find_optional_tool`.

---

## What I recommend cutting

- **The Ollama path** — decided (0.2).
- **Chasing loudness sophistication** — handoff §15 is right, no further work.
- **Whisper over the full film when subtitles exist** — it is the slowest pass in the system and
  subtitles already provide better-aligned text. Keep full-film Whisper only as the no-subtitle
  fallback, plus the targeted ±6 s windows from 4.1.
- **The `sensitivity` control** — it now only affects loudness, which is corroboration-only after
  3.6. Replace it with the per-category severity/confidence filters the store already has.

## If you only do five things

1. Fix the scene timestamp bug (0.1) — one line, verified, currently invalidates the whole visual path.
2. Restore the frontend build and serialize the pipeline (0.3, 0.4).
3. Build `scan_report` and annotate three movies (1.1, 1.2) — you cannot tune what you cannot measure.
4. Ship the region/fusion model (2.1, 2.2) — the difference between a tool that saves time and one that costs it.
5. Fix profanity mute timing with Whisper DTW (4.1) — the one feature where error is directly audible.

Phases 0–2 are roughly two weeks and take the project from "does not build" to "genuinely useful".
Phase 3 onward is tuning and polish against a benchmark that will, by then, exist.
