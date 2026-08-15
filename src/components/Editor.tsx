import { useStore, deriveEdits } from "../store";
import { useShortcuts } from "../hooks/useShortcuts";
import { fmtBytes, fmtTime } from "../lib/format";
import { BUILD_STAMP } from "../lib/buildStamp";
import Player from "./Player";
import Timeline from "./Timeline";
import SegmentsPanel from "./SegmentsPanel";
import TransportBar from "./TransportBar";
import ExportOverlay from "./ExportOverlay";

export default function Editor() {
  useShortcuts();
  const info = useStore((s) => s.info);
  const reset = useStore((s) => s.reset);
  const exportMovie = useStore((s) => s.exportMovie);
  const keyframesReady = useStore((s) => s.keyframesReady);
  const keyframesError = useStore((s) => s.keyframesError);
  const analysisError = useStore((s) => s.analysisError);
  const analyzing = useStore((s) => s.analyzing);
  const rebuildingPreview = useStore((s) => s.rebuildingPreview);
  const waveform = useStore((s) => s.waveform);
  const waveformPct = useStore((s) => s.waveformPct);
  const analysisPct = useStore((s) => s.analysisPct);
  const scans = useStore((s) => s.scans);
  const editCount = useStore((state) => {
    const edits = deriveEdits({
      events: state.events,
      eventStatus: state.eventStatus,
      manualCuts: state.manualCuts,
    });
    return edits.cuts.length + edits.mutes.length;
  });

  if (!info) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3">
        <p className="text-sm text-dust">This movie did not finish opening.</p>
        <button
          onClick={reset}
          className="rounded px-3 py-1.5 text-sm text-glow hover:bg-seam"
        >
          ← New movie
        </button>
      </div>
    );
  }

  const exportBlocked =
    !keyframesReady || analyzing || rebuildingPreview;
  const exportTitle = rebuildingPreview
    ? "Wait for the compatible preview to finish"
    : analyzing
      ? "Wait for the loudness pass to finish"
      : keyframesReady
        ? "Export from the untouched original movie"
        : (keyframesError ?? "Preparing the lossless export map");

  const statusBits = [
    !waveform && waveformPct > 0
      ? `Waveform ${Math.floor(waveformPct)}%`
      : null,
    analyzing ? `Loudness ${Math.floor(analysisPct)}%` : null,
    scans.text.running ? `Text ${Math.floor(scans.text.pct)}%` : null,
    scans.audio.running ? `Sound ${Math.floor(scans.audio.pct)}%` : null,
    scans.guide.running ? "Guide…" : null,
  ].filter(Boolean);

  const banner = keyframesError || analysisError;

  return (
    <div className="relative flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b border-seam bg-bay/60 pr-4 pl-20"
      >
        <button
          onClick={reset}
          className="rounded px-2 py-1 text-[11px] text-faint transition-colors hover:bg-seam hover:text-glow"
          aria-label="Close this movie"
        >
          ← New movie
        </button>
        <p className="pointer-events-none shrink-0 font-mono text-[10px] text-faint">
          {BUILD_STAMP}
        </p>
        <p className="pointer-events-none min-w-0 flex-1 truncate text-center text-sm font-medium text-glow">
          {info.fileName}
          <span className="ml-3 font-mono text-[10px] text-faint">
            {info.width}×{info.height} · {info.videoCodec} · {fmtTime(info.duration)} ·{" "}
            {fmtBytes(info.sizeBytes)}
          </span>
        </p>
        <button
          disabled={exportBlocked}
          onClick={() => void exportMovie()}
          title={exportTitle}
          className="rounded-md bg-glow px-4 py-1.5 text-sm font-semibold text-well transition-colors hover:bg-white disabled:cursor-wait disabled:opacity-45"
        >
          {keyframesReady
            ? `Export clean copy${editCount > 0 ? ` (${editCount})` : ""}`
            : "Preparing export…"}
        </button>
      </header>

      {banner && (
        <p className="shrink-0 border-b border-flare/30 bg-flare/10 px-4 py-1.5 text-center text-[11px] text-glow">
          {banner}
        </p>
      )}
      {statusBits.length > 0 && (
        <p className="shrink-0 border-b border-seam bg-bay/40 px-4 py-1 text-center font-mono text-[10px] text-faint">
          {statusBits.join(" · ")}
        </p>
      )}

      <div className="flex min-h-0 flex-1">
        <main className="flex min-w-0 flex-1 flex-col">
          <Player />
          <Timeline />
          <TransportBar />
        </main>
        <SegmentsPanel />
      </div>

      <ExportOverlay />
    </div>
  );
}
