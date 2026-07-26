# Videofy

**Movie night, minus the jump scares.**

Videofy is a desktop app that listens to a movie’s soundtrack for the audio signature of a jump scare — a sudden loudness spike out of quiet — lets you review each detected moment, and exports a copy with those parts cut out.

The export is a **lossless stream copy**: same picture quality, essentially the same file size, no re-encoding.

## Features

- **Automatic detection** — finds jump-scare candidates from loudness spikes (not every loud action scene)
- **Review & edit** — cut, ignore, multi-select, bulk actions, intensity sort/filter
- **Manual cuts** — mark your own in/out points when detection isn’t enough
- **JKL shuttle** — reverse / stop / forward with speed doubling (2×, 4×, 8×)
- **Stereo waveform timeline** — mirrored L/R peak lanes for precise scrubbing
- **Fast preview** — 540p hardware-encoded proxy so any container/codec scrubs smoothly
- **Lossless export** — ffmpeg concat demuxer with `-c copy`; cuts snap to keyframes so scary frames never leak back in
- **Per-file cache** — analysis, keyframes, and proxy are cached; re-opening is instant

## Requirements

| Dependency | Why | Notes |
| --- | --- | --- |
| **ffmpeg** + **ffprobe** | Probe, analyze loudness, build proxy, export | Must be on your `PATH` (or in a standard Homebrew/MacPorts location) |
| **macOS** (primary) | Desktop shell | Built and tested on Apple Silicon; Windows/Linux may work via Tauri but aren’t first-class yet |
| **Node.js 22+** | Frontend tooling | Only needed to build from source |
| **Rust** (stable) | Tauri backend | Only needed to build from source |

### Install ffmpeg (required to run the app)

```sh
# macOS
brew install ffmpeg

# Ubuntu / Debian
sudo apt install ffmpeg

# Windows (Chocolatey)
choco install ffmpeg
```

Confirm both tools are available:

```sh
ffmpeg -version
ffprobe -version
```

> **Note:** If you launch the app from Finder, it may not inherit your shell `PATH`. Videofy also looks in `/opt/homebrew/bin`, `/usr/local/bin`, and `/opt/local/bin` for ffmpeg/ffprobe.

## Install from source

```sh
git clone https://github.com/inspiretelapps/videofy.git
cd videofy
npm install
```

### Development (hot reload)

```sh
npm run tauri dev
```

### Production build (`.app` / installer)

```sh
npm run tauri build
```

Artifacts land under:

```
src-tauri/target/release/bundle/macos/Videofy.app
src-tauri/target/release/bundle/dmg/Videofy_*.dmg
```

Open the app:

```sh
open src-tauri/target/release/bundle/macos/Videofy.app
```

### Tests

```sh
cd src-tauri
cargo test
```

## Usage

1. Drop a video onto the window, or click to pick a file (mp4, mkv, mov, and most common containers).
2. Wait for proxy generation and loudness analysis (first open only; results are cached).
3. Scrub the timeline / use keyboard shortcuts to review each detection.
4. **Enter** to cut a scare, **Delete** to ignore it, or mark manual cuts with **I** / **O**.
5. Press **E** (or use Export) to write a clean copy next to the original.

### Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Space` | Play / pause |
| `J` `K` `L` | Shuttle: reverse / stop / forward — press `J` or `L` again for 2×, 4×, 8× |
| `[` `]` | Previous / next detected scare |
| `Enter` | Cut the selected scare |
| `Delete` | Ignore the selected scare / remove manual cut |
| `I` `O` | Mark a manual cut in / out |
| `,` `.` | Frame step |
| `E` | Export clean copy |
| Scroll | Zoom timeline |

## How it works

- **Detection** — ffmpeg’s EBU R128 meter samples momentary loudness every 100 ms. A candidate fires when it jumps **8–16 LU** (by sensitivity) above a rolling **20-second** baseline, so steady-loud action scenes don’t trigger it.
- **Preview** — a 540p hardware-encoded proxy is generated on import so scrubbing is instant in the webview. Analysis, keyframes, and the proxy are cached per file.
- **Export** — ffmpeg concat demuxer with `-c copy`. Keep-segment starts snap **forward** to the next keyframe, so a cut only ever removes slightly more than marked — scary frames never leak back in.

## Stack

| Layer | Tech |
| --- | --- |
| Shell | [Tauri 2](https://tauri.app) (Rust) |
| UI | React 19 · TypeScript · Tailwind CSS v4 · Zustand |
| Media | ffmpeg / ffprobe (external) |

## Project layout

```
videofy/
├── src/                 # React frontend
│   ├── components/      # Editor, player, timeline, panels
│   ├── hooks/           # Keyboard shortcuts
│   └── store.ts         # App state (Zustand)
├── src-tauri/           # Rust / Tauri backend
│   └── src/             # Probe, analysis, proxy, waveform, export
├── package.json
└── README.md
```

## Privacy

Videofy runs entirely on your machine. Media is processed locally via ffmpeg; nothing is uploaded.

## Contributing

Issues and pull requests are welcome. For larger changes, open an issue first so we can align on approach.

## License

No license file is published yet — all rights reserved until one is added. If you want to use this in a product or redistribute it, open an issue.
