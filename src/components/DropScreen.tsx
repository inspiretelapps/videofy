import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useStore } from "../store";
import { fmtBytes } from "../lib/format";

const VIDEO_EXTENSIONS = [
  "mp4", "mkv", "mov", "m4v", "avi", "webm", "ts", "m2ts", "mpg", "mpeg", "wmv", "flv",
];
const SUBTITLE_EXTENSIONS = ["srt", "vtt", "ass", "ssa"];

function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

export default function DropScreen() {
  const stage = useStore((s) => s.stage);
  const importError = useStore((s) => s.importError);
  const openFile = useStore((s) => s.openFile);
  const info = useStore((s) => s.info);
  const proxyPct = useStore((s) => s.proxyPct);
  const analysisPct = useStore((s) => s.analysisPct);
  const waveformPct = useStore((s) => s.waveformPct);
  const skipDetection = useStore((s) => s.skipDetection);
  const setSkipDetection = useStore((s) => s.setSkipDetection);
  const [hovering, setHovering] = useState(false);
  const [subtitlePath, setSubtitlePath] = useState<string | null>(null);

  const importing = stage === "importing";

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "over") {
        setHovering(true);
      } else if (event.payload.type === "drop") {
        setHovering(false);
        const path = event.payload.paths.find((p) =>
          VIDEO_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""),
        );
        const subtitle = event.payload.paths.find((p) =>
          SUBTITLE_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""),
        );
        if (path && !importing) void openFile(path, subtitle ?? subtitlePath);
        else if (subtitle && !importing) setSubtitlePath(subtitle);
      } else {
        setHovering(false);
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [openFile, importing, subtitlePath]);

  const browse = useCallback(async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "Movies", extensions: VIDEO_EXTENSIONS }],
    });
    if (typeof path === "string") void openFile(path, subtitlePath);
  }, [openFile, subtitlePath]);

  const browseSubtitle = useCallback(async () => {
    const path = await open({
      multiple: false,
      filters: [
        { name: "Subtitle files", extensions: SUBTITLE_EXTENSIONS },
      ],
    });
    if (typeof path === "string") setSubtitlePath(path);
  }, []);

  return (
    <div className="flex h-full flex-col items-center justify-center px-10">
      <div className="rise-in w-full max-w-2xl">
        <p className="mb-3 text-center font-mono text-[11px] tracking-[0.3em] text-faint uppercase">
          Videofy
        </p>
        <h1 className="text-center font-display text-5xl font-semibold tracking-tight text-glow">
          Movie night,
          <br />
          <span className="text-flare">minus the jump scares.</span>
        </h1>
        <p className="mx-auto mt-5 max-w-md text-center text-[15px] leading-relaxed text-dust">
          Drop a movie in. Videofy checks captions, dialogue, sound events, and
          optional human guides so you can review and remove unsuitable moments
          quickly.
        </p>

        {!importing ? (
          <>
            <label className="mx-auto mt-8 flex max-w-md cursor-pointer items-start gap-3 rounded-lg border border-seam bg-bay/40 px-4 py-3 text-left transition-colors hover:border-faint">
              <input
                type="checkbox"
                checked={skipDetection}
                onChange={(event) => setSkipDetection(event.target.checked)}
                className="mt-0.5 h-4 w-4 accent-[var(--color-flare)]"
              />
              <span>
                <span className="block text-sm font-medium text-glow">
                  Quick manual edit
                </span>
                <span className="mt-0.5 block text-xs leading-relaxed text-faint">
                  Skip content detection and open only the editor, waveform, and
                  export tools.
                </span>
              </span>
            </label>
            <button
              onClick={browse}
              className={`mt-5 block w-full rounded-xl border-2 border-dashed px-8 py-14 text-center transition-colors ${
                hovering
                  ? "border-flare bg-flare/10"
                  : "border-seam bg-bay/40 hover:border-faint"
              }`}
            >
              <span className="block text-lg font-medium text-glow">
                {hovering ? "Drop it here" : "Drop a movie here"}
              </span>
              <span className="mt-1 block text-sm text-dust">
                or click to choose a file
              </span>
            </button>
            <div className="mt-3 flex items-center justify-center gap-2 text-xs">
              <button
                onClick={browseSubtitle}
                className="rounded-md border border-seam bg-bay/40 px-3 py-1.5 text-dust hover:border-faint hover:text-glow"
              >
                {subtitlePath ? "Change subtitle" : "Add subtitle file"}
              </button>
              {subtitlePath ? (
                <>
                  <span
                    className="max-w-64 truncate text-faint"
                    title={subtitlePath}
                  >
                    {fileName(subtitlePath)}
                  </span>
                  <button
                    onClick={() => setSubtitlePath(null)}
                    className="text-faint hover:text-flare"
                    aria-label="Remove selected subtitle"
                  >
                    ×
                  </button>
                </>
              ) : (
                <span className="text-faint">
                  Optional · matching subtitles beside the movie are found automatically
                </span>
              )}
            </div>
            {importError && (
              <p className="mt-4 text-center text-sm text-flare">{importError}</p>
            )}
            <div className="mt-10 flex justify-center gap-10 text-center">
              {(skipDetection
                ? [
                    ["Open", "prepare preview and waveform"],
                    ["Edit", "mark manual cuts on the timeline"],
                    ["Export", "save your edited copy"],
                  ]
                : [
                    ["Scan", "text, sound, guides"],
                    ["Review", "cut, mute, or keep each clue"],
                    ["Export", "clean copy for movie night"],
                  ]
              ).map(([title, sub]) => (
                <div key={title} className="w-40">
                  <p className="font-display text-sm font-semibold text-glow">{title}</p>
                  <p className="mt-1 text-xs leading-snug text-faint">{sub}</p>
                </div>
              ))}
            </div>
          </>
        ) : (
          <div className="mt-10 rounded-xl border border-seam bg-bay/60 px-8 py-8">
            <p className="truncate text-center text-sm font-medium text-glow">
              {info ? info.fileName : "Reading file…"}
            </p>
            {info && (
              <p className="mt-1 text-center font-mono text-[11px] text-faint">
                {info.width}×{info.height} · {info.videoCodec} · {fmtBytes(info.sizeBytes)}
              </p>
            )}
            <div className="mt-6 space-y-4">
              {!skipDetection && (
                <ProgressRow label="Preparing audio baseline" pct={analysisPct} />
              )}
              <ProgressRow label="Tracing the waveform" pct={waveformPct} />
              <ProgressRow label="Building preview" pct={proxyPct} />
            </div>
            <p className="mt-6 text-center text-xs text-faint">
              Long movies take a few minutes the first time. Re-opening is instant.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function ProgressRow({ label, pct }: { label: string; pct: number }) {
  return (
    <div>
      <div className="mb-1.5 flex items-baseline justify-between">
        <span className="text-xs text-dust">{label}</span>
        <span className="font-mono text-[11px] text-faint">{Math.floor(pct)}%</span>
      </div>
      <div className="h-1 overflow-hidden rounded-full bg-seam">
        <div
          className="h-full rounded-full bg-flare transition-[width] duration-300"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}
