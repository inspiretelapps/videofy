# Videofy

**A faster way for a parent to make a child-friendly copy of a movie.**

Videofy is a local-first macOS/Tauri editor that finds potentially unsuitable
moments, explains why each moment was flagged, and lets you mark it as
**Cut**, **Mute**, or **Keep** before exporting a cleaned copy.

It does not treat loudness as proof that a scene is scary. Loudness is now one
weak clue in a hybrid scan that also examines text, semantic sound events,
representative scene frames, accessibility tracks, and optional human guide
timestamps.

## What the scanner checks

| Pass | What it contributes |
| --- | --- |
| Embedded subtitles / SDH | Profanity, threats, violence, sexual references, substances, bullying, and captions such as `[SCREAMS]` |
| Audio description | Narrated visual actions that ordinary subtitles can miss |
| Local Whisper fallback | English dialogue when usable text subtitles are unavailable; includes word-level timing for short profanity mutes |
| YAMNet sound classifier | Screams, crying, growls, gunshots, explosions, breaking glass, crashes, and similar semantic sound clues |
| Scene scan | Frames at scene changes, checked for conservative visual risk clues |
| Does the Dog Die? | Optional human-written timestamped ratings and Scene Alerts, depending on the API tier |
| SRT / VTT / SKP import | Published or hand-authored timing ranges, with a manual offset and conservative automatic jump-scare alignment |
| Loudness | Low-confidence sudden-impact evidence only |

Overlapping clues for the same category and moment are fused into one event.
Severity and detector confidence remain separate, and the evidence behind each
event is visible in the review panel.

Current categories are frightening content, violence, sexual material, nudity,
language, substances, bullying, and other disturbing material.

## Review and editing

- Filter by category and minimum severity.
- Sort by time, severity, or confidence.
- Inspect the evidence and source for every result.
- Mark an event as **Cut**, **Mute**, or **Keep**.
- Bulk-apply a decision to checked or visible pending events.
- Add precise manual IN/OUT cuts.
- Import guide timestamps or look up a title through Does the Dog Die?.
- Keep review decisions and manual cuts when the movie is reopened.
- Change loudness sensitivity without discarding decisions made on the other
  scanner passes.

The waveform, proxy, model results, and media analysis are cached per source
file. The editor opens as soon as the proxy and basic analysis are ready;
semantic results appear progressively as the deeper passes complete.

## Requirements

| Dependency | Why | Notes |
| --- | --- | --- |
| **ffmpeg** + **ffprobe** | Media probing, track extraction, proxy, scanning, and export | Required at runtime |
| **macOS** | Primary desktop target | Apple Silicon is the primary tested target |
| **Node.js 22+** | Frontend tooling | Build-time only |
| **Rust stable** | Tauri backend | Build-time only |
| **Does the Dog Die? API key** | Optional external guide lookup | Timestamp ratings require the appropriate provider tier |

Install ffmpeg on macOS:

```sh
brew install ffmpeg
```

Videofy also searches `/opt/homebrew/bin`, `/usr/local/bin`, and
`/opt/local/bin`, which helps when the app is launched from Finder.

## First-run models

The semantic models are downloaded into Videofy’s application-data `models`
folder the first time they are needed:

- YAMNet ONNX and its AudioSet labels for the sound-event pass.
- Whisper `base.en` only when transcription or an audio-description track is
  needed.

Downloads are atomic and progress is shown in the scanner coverage panel.
Whisper is English-only in the current implementation.

## Install and run from source

```sh
git clone https://github.com/inspiretelapps/videofy.git
cd videofy
npm install
npm run tauri dev
```

Build a production app:

```sh
npm run tauri build
```

Artifacts are written under `src-tauri/target/release/bundle/`.

Run validation:

```sh
npm run build
cd src-tauri
cargo test
```

## Usage

1. Drop a movie onto the window or choose a file.
2. Start reviewing when the editor opens; Text, Sound, and Picture results will
   continue to appear in the coverage panel.
3. Expand an event to see why it was flagged.
4. Choose **Cut**, **Mute**, or **Keep**, and add manual IN/OUT cuts where
   required.
5. Optionally import an SRT/VTT/SKP timing file or use a Does the Dog Die? API
   key to fetch timestamped guide entries.
6. Export the cleaned copy.

### Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Space` | Play / pause |
| `J` `K` `L` | Reverse / stop / forward shuttle; repeat for 2×, 4×, 8× |
| `[` `]` | Previous / next visible event or manual cut |
| `Enter` | Toggle Cut on the selected event |
| `Delete` | Keep the selected event, or remove a selected manual cut |
| `I` `O` | Mark a manual cut in / out |
| `,` `.` | Frame step |
| `E` | Export |
| Scroll | Zoom or pan the timeline |

## Export behavior

Video is stream-copied, and retained sections begin at safe keyframes so frames
from a removed range do not leak back into the result.

- With cuts only, video, audio, and compatible subtitle streams are copied
  without re-encoding.
- If any mute is used, video and subtitles are still copied, but audio is
  re-encoded to AAC so the selected ranges can be silenced.

## Important limitations

Videofy is a review accelerator, not a child-safety certification. Automated
models can miss unsuitable content and can flag harmless scenes. Visual
heuristics are deliberately low-confidence review clues. Subtitle wording,
alternate cuts, logos, frame rates, and regional editions can shift imported
timestamps, so imported ranges must be reviewed.

The automatic guide alignment currently estimates a single timeline offset
from matching jump-scare impacts. The manual offset remains available when
there are too few reliable anchors or the edition differs more substantially.

## Privacy

Movie frames, audio, subtitles, and transcripts are processed on the machine.
The app makes network requests only to download its model files and, when the
user explicitly requests a guide lookup, to send the entered title/year to
Does the Dog Die?.

The Does the Dog Die? API key is stored in the app webview’s local storage. No
movie media is uploaded by Videofy.

## Stack

| Layer | Technology |
| --- | --- |
| Desktop shell | Tauri 2 / Rust |
| UI | React 19, TypeScript, Tailwind CSS 4, Zustand |
| Media | ffmpeg / ffprobe |
| Speech | whisper.cpp through `whisper-rs` |
| Semantic audio | YAMNet ONNX through ONNX Runtime |

## License

[MIT](LICENSE)
