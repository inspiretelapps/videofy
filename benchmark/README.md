# Detection benchmark

Every threshold in the detectors is currently a principled guess. This directory
is where they stop being guesses.

## 1. Annotate a movie

Copy `annotations.example.json`, name it after the film, and list the ranges you
would actually cut or mute for a six-year-old. Watch at 2x with the timeline
open; a feature takes about an hour.

Times accept `HH:MM:SS`, `MM:SS`, or plain seconds. Categories are
`frightening`, `violence`, `sexual`, `nudity`, `language`, `substances`,
`bullying`, `disturbing`. Leave `categories` empty if you only care that
*something* was there.

Annotate what you would act on, not everything a detector might notice — the
score is measuring whether the tool saves you time, not whether it is thorough.

## 2. Run the harness

```sh
cd src-tauri
cargo run --release --bin scan_report -- "/path/to/Movie.mkv" \
  --annotations ../benchmark/movie.json \
  --json ../benchmark/movie-report.json
```

Useful flags:

- `--skip loudness,text,audio` — omit passes while iterating on one
- `--labels 60` — show more of the YAMNet score histogram
- `--quiet` — no per-pass progress
- `--profanity off|strong|medium|mild` — which language tier to mute (default
  `medium`); re-run across tiers to see what each one costs you in false
  positives before choosing the app's setting
- `--no-verify` — skip Whisper confirmation of subtitle mute timing. Faster,
  but mute ranges revert to guessing where in the cue the word falls

First run downloads YAMNet (~16 MB), and Whisper (~140 MB) if the film has no
text subtitle track. Use `--skip text` to avoid the Whisper path entirely.

## 3. Read the output

**Events per hour, per source.** This is the number that decides whether the
review list is usable. If one source dominates the total, it is miscalibrated,
not productive.

**Recall vs precision.** Recall is what you missed — weight it heavily. Precision
is what it cost you to catch that. A source with high recall and 5% precision is
buying safety with your evening.

**Median start-boundary error.** How far off the cut points are. Large values
mean the padding constants need work, not the detection.

**Per-event ranges.** The JSON report lists every event with its range,
confidence and source. For mutes, compare a `--no-verify` run against a normal
one: the difference is how wrong the subtitle estimate was.

**YAMNet label histogram.** For each label, how many 0.48 s frames scored above
each threshold. Set `RISK_RULES` thresholds in `src-tauri/src/audio_events.rs`
where the count drops to something you would genuinely review — then re-run and
check recall did not fall with it.

## 4. Three movies is enough to start

Pick ones you know well and that differ: one animated, one live-action
adventure, one you have already decided is borderline. Three gives you a signal;
ten to twenty gives you confidence. Re-run after every threshold change — the
whole point is that this loop is cheap.
