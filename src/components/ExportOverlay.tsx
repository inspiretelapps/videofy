import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useStore } from "../store";
import { fmtBytes, fmtSeconds } from "../lib/format";

export default function ExportOverlay() {
  const exporting = useStore((s) => s.exporting);
  const dismissExport = useStore((s) => s.dismissExport);
  if (!exporting) return null;

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-well/80 backdrop-blur-sm">
      <div className="rise-in w-[26rem] rounded-xl border border-seam bg-bay p-6 shadow-2xl">
        {exporting.running ? (
          <>
            <h3 className="font-display text-lg font-semibold text-glow">
              Exporting clean copy
            </h3>
            <p className="mt-1 text-sm text-dust">
              Keeping the original picture while applying approved cuts and
              audio mutes.
            </p>
            <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-seam">
              <div
                className="ember-pulse h-full rounded-full bg-flare transition-[width] duration-200"
                style={{ width: `${exporting.pct}%` }}
              />
            </div>
            <p className="mt-2 text-right font-mono text-[11px] text-faint">
              {Math.floor(exporting.pct)}%
            </p>
          </>
        ) : exporting.error ? (
          <>
            <h3 className="font-display text-lg font-semibold text-flare">
              Export didn't finish
            </h3>
            <p className="mt-2 max-h-40 overflow-y-auto font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-dust">
              {exporting.error}
            </p>
            <div className="mt-5 flex justify-end">
              <Button onClick={dismissExport}>Close</Button>
            </div>
          </>
        ) : exporting.result ? (
          <>
            <h3 className="font-display text-lg font-semibold text-glow">Saved.</h3>
            <p className="mt-1 text-sm leading-relaxed text-dust">
              Removed{" "}
              <span className="text-glow">
                {fmtSeconds(exporting.result.removedDuration)}
              </span>{" "}
              across {exporting.result.segments} kept segment
              {exporting.result.segments === 1 ? "" : "s"} ·{" "}
              {exporting.result.mutedDuration > 0
                ? `${fmtSeconds(exporting.result.mutedDuration)} muted · `
                : ""}
              {fmtBytes(exporting.result.sizeBytes)} · original quality.
            </p>
            <p className="mt-3 truncate font-mono text-[11px] text-faint">
              {exporting.result.outPath}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button
                subtle
                onClick={() => void revealItemInDir(exporting.result!.outPath).catch(() => {})}
              >
                Show in Finder
              </Button>
              <Button onClick={dismissExport}>Done</Button>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}

function Button({
  children,
  onClick,
  subtle,
}: {
  children: React.ReactNode;
  onClick: () => void;
  subtle?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md px-4 py-1.5 text-sm font-medium transition-colors ${
        subtle
          ? "text-dust hover:bg-seam hover:text-glow"
          : "bg-glow text-well hover:bg-white"
      }`}
    >
      {children}
    </button>
  );
}
