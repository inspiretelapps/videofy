import { useStore, deriveCuts } from "../store";
import { useShortcuts } from "../hooks/useShortcuts";
import { fmtBytes, fmtTime } from "../lib/format";
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
  const cutCount = useStore(
    (s) =>
      deriveCuts({
        analysis: s.analysis,
        candidateStatus: s.candidateStatus,
        manualCuts: s.manualCuts,
      }).length,
  );

  if (!info) return null;

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
        <p className="pointer-events-none min-w-0 flex-1 truncate text-center text-sm font-medium text-glow">
          {info.fileName}
          <span className="ml-3 font-mono text-[10px] text-faint">
            {info.width}×{info.height} · {info.videoCodec} · {fmtTime(info.duration)} ·{" "}
            {fmtBytes(info.sizeBytes)}
          </span>
        </p>
        <button
          onClick={() => void exportMovie()}
          className="rounded-md bg-glow px-4 py-1.5 text-sm font-semibold text-well transition-colors hover:bg-white"
        >
          Export clean copy{cutCount > 0 ? ` (${cutCount})` : ""}
        </button>
      </header>

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
