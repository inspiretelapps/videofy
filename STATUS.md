# Videofy — current state

Last updated: 2026-07-27. Read this first when picking the project back up.

Companion documents: `ACCURACY.md` (why the detectors are noisy and how to fix
it), `benchmark/README.md` (how to run the calibration harness),
`DETECTION_IMPLEMENTATION_PLAN.md` (the plan, now partly delivered — see below),
`DETECTION_IMPLEMENTATION_SUMMARY.md` (what the detection overhaul contains).

## Where things stand

Commit `d248cec` is pushed to `main`; it adds Quick manual edit and removes the
low-quality Picture detector. The current workspace adds local-first external
subtitle support: attach a subtitle while opening a movie, or let the scanner
find a safely matching English sidecar file beside it. Online subtitle lookup
is deliberately not part of the automatic scan. The workspace passes all 35
Rust tests and the frontend/production builds; the signed bundle is installed
and launch-checked at `/Applications/Videofy.app`.

Six commits since the last release point:

| commit | what |
| --- | --- |
| `475c4b3` | detection overhaul: subtitles, Whisper, YAMNet, scenes, guides, mute export |
| `05943eb` | app bundling, timeline responsiveness, zoom, first audio attempt |
| `dbd0523` | timeline click robustness, media diagnostics, devtools enabled |
| `c9899c2` | scrub lock, audio measurement, output device selector |
| `aceb319` | removed the risky Web Audio tap, added the test tone |
| `202e23e` | **stripped the chapter track from the proxy** — the audio fix |

## The one thing that needs verifying

**Audio is fixed in theory but unconfirmed in practice.**

The proxy contained a third stream — a `text` chapter track that ffmpeg's mp4
muxer copies from the source, and which survives explicit stream mapping.
WebKit responded by playing the picture and ignoring the sound; VLC is more
tolerant, which is why the same file played fine there. `export.rs` already
stripped chapters; `proxy.rs` did not. Fixed in `202e23e`, cache bumped to
`proxy-v3` so old proxies regenerate.

To confirm: open a movie (the first one will spend a few minutes re-encoding
its proxy) and press play.

- Sound plays → done; the diagnostic readout and Test sound button in
  `src/components/Player.tsx` can be removed.
- Still silent → read the overlay at the top-left of the video. `tracks=0`
  means the webview still cannot see an audio track and the container is still
  wrong. `tracks=1` with no sound means the problem is downstream of the media
  element; open the Console (right-click outside the timeline → Inspect
  Element; devtools is enabled in the release build).

What is already ruled out, and does not need re-testing: the webview's audio
stack, autoplay policy, output device routing and system audio. A Web Audio
test tone plays fine, which eliminates all of them. Do not route the video
through Web Audio to investigate — `createMediaElementSource` silences
cross-origin media in Safari, and the video is served from `asset.localhost`
while the page runs on `tauri.localhost`. That was tried and reverted.

## Known open items

**Does the Dog Die? access is working, but the current key is Free-tier.**
Authenticated lookups correctly matched Moana (2016) and Moana 2 (2024) and
returned whole-film topic statistics. The `/ratings` endpoint returned 403 for
both, so this key cannot retrieve community timestamps or professional Scene
Alerts. The app should continue treating this as an optional guide source;
timestamped use requires Startup or Pro API access.

**Detection accuracy — the big one.** Every threshold is still a principled
guess. Nothing has been run against a real annotated movie. This blocks any
honest tuning, and it is the user-visible complaint ("picking up way too
much"). `ACCURACY.md` explains the approach; `benchmark/README.md` covers the
mechanics. The loop is:

```sh
cd src-tauri
cargo run --release --bin scan_report -- "/path/to/Movie.mkv" \
  --annotations ../benchmark/movie.json --json ../benchmark/movie-report.json
```

Two or three annotated films is enough to start. The JSON report lists every
event with its range, confidence and source, plus a YAMNet score histogram for
picking thresholds from data.

**`fuseEvents` is O(N²)** (`src/store.ts`), called on every scan result. Measured
at 13 ms for 500 events, 1373 ms for 8000. Deliberately deferred: the YAMNet
recalibration should cut event volume enough that it never bites. Decide after
`scan_report` shows real counts. A ~30 minute fix (bucket clusters by category,
hoist the per-comparison Set) gets most of the win; the full sliding-window
rewrite is half a day and wants a differential test against the current
implementation to avoid silently changing fusion semantics.

**Subtitle/audio mismatch.** Mute timing is confirmed against the soundtrack
with windowed Whisper, and unconfirmed mutes are kept and marked rather than
dropped. Whether that threshold is right is unmeasured.

**Panel row cap.** The review list renders 200 rows at a time with an explicit
"show more". Fine as a safety net, but it is a symptom of event volume, not a
cure.

## Things worth not relearning

- The timeline panel, not the timeline itself, was what made clicks feel dead:
  3000 events meant 51k DOM nodes and 160 ms to respond to a click. Fixed by
  windowing the list, memoising the filter/sort, and throttling playhead pushes
  to 25/sec.
- `setPointerCapture` can throw in a webview. It now runs after the seek and is
  guarded, because when it threw it took the seek down with it silently.
- Adding a second binary to the crate broke app bundling — Tauri shipped
  `scan_report` as `Videofy.app`. `mainBinaryName` in `tauri.conf.json` plus
  explicit `[[bin]]` targets fix it. Double-clicking produced no window at all.
- `HeadlessHost` must use the same bundle identifier as `tauri.conf.json`, or
  `scan_report` warms a different cache than the app. There is a test for it.
- Diagnose by inspecting the artifact, not by reasoning about the platform.
  The audio bug took four rounds; `ffprobe` on the proxy would have shown the
  stray stream in the first.
