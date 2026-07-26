# Videofy

Movie night, minus the jump scares.

Videofy is a desktop app that listens to a movie's soundtrack for the audio
signature of a jump scare — a sudden loudness spike out of quiet — lets you
review each detected moment, and exports a copy with those parts cut out.
The export is a lossless stream copy: same picture quality, essentially the
same file size, no re-encoding.

## How it works

- **Detection** — ffmpeg's EBU R128 meter samples momentary loudness every
  100 ms. A candidate fires when it jumps 8–16 LU (by sensitivity) above the
  rolling 20-second baseline, so steady-loud action scenes don't trigger it.
- **Preview** — a 540p hardware-encoded proxy is generated on import, so any
  container/codec scrubs instantly in the webview. Analysis, keyframes, and
  the proxy are cached per file; re-opening is instant.
- **Export** — ffmpeg concat demuxer with `-c copy`. Keep-segment starts snap
  forward to the next keyframe, so a cut only ever removes slightly more than
  marked — scary frames never leak back in.

## Keyboard

| Key | Action |
| --- | --- |
| `Space` | play / pause |
| `[` `]` | previous / next detected scare |
| `Enter` | cut the selected scare |
| `Delete` | ignore the selected scare / remove manual cut |
| `I` `O` | mark a manual cut in / out |
| `,` `.` | frame step |
| `E` | export clean copy |
| scroll | zoom timeline |

## Development

Requires Node 22+, Rust, and ffmpeg/ffprobe on PATH (`brew install ffmpeg`).

```sh
npm install
npm run tauri dev    # run the app
npm run tauri build  # bundle a .app / installer
cargo test           # detection + segment-planning tests (in src-tauri)
```

Stack: Tauri 2, React 18 + TypeScript + Tailwind v4, zustand; Rust backend
shells out to ffmpeg for probing, analysis, proxy, and export.
