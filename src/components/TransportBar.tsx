import { useStore } from "../store";
import { fmtTime } from "../lib/format";

export default function TransportBar() {
  const shuttle = useStore((s) => s.shuttle);
  const setShuttle = useStore((s) => s.setShuttle);
  const playhead = useStore((s) => s.playhead);
  const seekTo = useStore((s) => s.seekTo);
  const pendingIn = useStore((s) => s.pendingIn);
  const info = useStore((s) => s.info);
  const zoomBy = useStore((s) => s.zoomBy);
  const zoomToFit = useStore((s) => s.zoomToFit);
  const view = useStore((s) => s.view);
  const playing = shuttle !== 0;
  const span = view.t1 - view.t0;

  return (
    <div className="flex h-12 shrink-0 items-center gap-4 border-t border-seam bg-bay/60 px-4">
      <div className="flex items-center gap-1">
        <IconButton label="Back 10 seconds" onClick={() => seekTo(playhead - 10)}>
          ⟲10
        </IconButton>
        <button
          onClick={() => setShuttle(playing ? 0 : 1)}
          aria-label={playing ? "Pause" : "Play"}
          className="mx-1 flex h-8 w-8 items-center justify-center rounded-full bg-glow text-well transition-transform hover:scale-105"
        >
          <span className="text-[13px] leading-none">{playing ? "❚❚" : "▶"}</span>
        </button>
        <IconButton label="Forward 10 seconds" onClick={() => seekTo(playhead + 10)}>
          10⟳
        </IconButton>
      </div>

      <div className="flex items-center gap-1">
        <IconButton label="Zoom out" onClick={() => zoomBy(2)}>
          −
        </IconButton>
        <button
          onClick={zoomToFit}
          title="Fit whole movie"
          className="min-w-14 rounded px-2 py-1 font-mono text-[11px] text-faint hover:bg-seam hover:text-glow"
        >
          {fmtTime(span)}
        </button>
        <IconButton label="Zoom in" onClick={() => zoomBy(0.5)}>
          +
        </IconButton>
      </div>

      <p className="font-mono text-sm tracking-tight text-glow tabular-nums">
        {fmtTime(playhead, true)}
        <span className="text-faint"> / {fmtTime(info?.duration ?? 0)}</span>
      </p>

      {shuttle !== 0 && shuttle !== 1 && (
        <p className="rounded bg-glow/10 px-2 py-0.5 font-mono text-[11px] text-glow">
          {shuttle < 0 ? "◂" : "▸"} {Math.abs(shuttle)}×
        </p>
      )}

      {pendingIn !== null && (
        <p className="rounded bg-amber/15 px-2 py-0.5 font-mono text-[11px] text-amber">
          IN {fmtTime(pendingIn, true)} — press O to finish the cut
        </p>
      )}

      <div className="ml-auto hidden items-center gap-3 font-mono text-[10px] text-faint lg:flex">
        <Hint k="J K L">shuttle</Hint>
        <Hint k="I / O">mark cut</Hint>
        <Hint k="⏎">cut selected</Hint>
        <Hint k="[ ]">prev / next clue</Hint>
        <Hint k="scroll">zoom</Hint>
      </div>
    </div>
  );
}

function IconButton({
  children,
  label,
  onClick,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      className="rounded px-2 py-1 font-mono text-[11px] text-dust transition-colors hover:bg-seam hover:text-glow"
    >
      {children}
    </button>
  );
}

function Hint({ k, children }: { k: string; children: React.ReactNode }) {
  return (
    <span>
      <kbd className="rounded border border-seam bg-well px-1 py-0.5">{k}</kbd>{" "}
      {children}
    </span>
  );
}
