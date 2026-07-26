import { useStore, type MinIntensity, type Sensitivity, type SortBy } from "../store";
import { fmtSeconds, fmtTime } from "../lib/format";
import type { ScareCandidate } from "../types";

const SENSITIVITY_HELP: Record<Sensitivity, string> = {
  strict: "Flags only the biggest loudness jumps — fewest false alarms.",
  balanced: "A middle ground that suits most movies.",
  sensitive: "Flags smaller jumps too — catches more, with more to review.",
};

export default function SegmentsPanel() {
  const analysis = useStore((s) => s.analysis);
  const analysisError = useStore((s) => s.analysisError);
  const analyzing = useStore((s) => s.analyzing);
  const sensitivity = useStore((s) => s.sensitivity);
  const changeSensitivity = useStore((s) => s.changeSensitivity);
  const candidateStatus = useStore((s) => s.candidateStatus);
  const manualCuts = useStore((s) => s.manualCuts);
  const bulkSetStatus = useStore((s) => s.bulkSetStatus);
  const removeManualCut = useStore((s) => s.removeManualCut);
  const selection = useStore((s) => s.selection);
  const sortBy = useStore((s) => s.sortBy);
  const setSortBy = useStore((s) => s.setSortBy);
  const minIntensity = useStore((s) => s.minIntensity);
  const setMinIntensity = useStore((s) => s.setMinIntensity);
  const checkedIds = useStore((s) => s.checkedIds);
  const showDetections = useStore((s) => s.showDetections);
  const toggleDetections = useStore((s) => s.toggleDetections);

  const candidates = analysis?.candidates ?? [];
  const shown = candidates
    .filter((c) => c.score >= minIntensity)
    .sort((a, b) =>
      sortBy === "intensity" ? b.score - a.score : a.peakTime - b.peakTime,
    );
  const hiddenCount = candidates.length - shown.length;
  const shownPending = shown.filter((c) => candidateStatus[c.id] === "pending");
  const checkedShown = checkedIds.filter((id) => shown.some((c) => c.id === id));
  const bulkIds = checkedShown.length > 0 ? checkedShown : shownPending.map((c) => c.id);
  const bulkLabel =
    checkedShown.length > 0
      ? `${checkedShown.length} selected`
      : `all ${shownPending.length} pending`;

  return (
    <aside className="flex w-72 shrink-0 flex-col border-l border-seam bg-bay/40">
      <div className="border-b border-seam px-4 pt-4 pb-3">
        <div className="flex items-baseline justify-between">
          <h2 className="font-display text-sm font-semibold tracking-wide text-glow">
            Detected scares
          </h2>
          <span className="flex items-baseline gap-2">
            <span className="font-mono text-[11px] text-faint">{candidates.length}</span>
            <button
              onClick={toggleDetections}
              className={`rounded px-1.5 py-0.5 text-[11px] transition-colors ${
                showDetections
                  ? "text-faint hover:bg-seam hover:text-glow"
                  : "bg-amber/15 text-amber hover:bg-amber/25"
              }`}
            >
              {showDetections ? "Hide" : "Show"}
            </button>
          </span>
        </div>

        {!showDetections && (
          <p className="mt-2 text-[11px] leading-snug text-dust">
            Detections are hidden and won't be cut on export — you're in manual
            mode. Mark cuts with <Kbd>I</Kbd> and <Kbd>O</Kbd>.
          </p>
        )}

        {showDetections && (
          <>
        <p className="mt-3 text-[10px] font-medium tracking-[0.15em] text-faint uppercase">
          Detection
        </p>
        <div className="mt-1 flex rounded-md border border-seam p-0.5">
          {(["strict", "balanced", "sensitive"] as Sensitivity[]).map((s) => (
            <button
              key={s}
              onClick={() => void changeSensitivity(s)}
              disabled={analyzing}
              className={`flex-1 rounded px-2 py-1 text-[11px] capitalize transition-colors ${
                sensitivity === s ? "bg-seam text-glow" : "text-faint hover:text-dust"
              }`}
            >
              {s}
            </button>
          ))}
        </div>
        <p className="mt-1.5 min-h-7 text-[11px] leading-snug text-faint">
          {SENSITIVITY_HELP[sensitivity]}
        </p>

        <div className="mt-2 flex gap-2">
          <Control label="Sort by">
            {(
              [
                ["time", "Time"],
                ["intensity", "Intensity"],
              ] as [SortBy, string][]
            ).map(([value, label]) => (
              <SmallToggle
                key={value}
                active={sortBy === value}
                onClick={() => setSortBy(value)}
              >
                {label}
              </SmallToggle>
            ))}
          </Control>
          <Control label="Show">
            {(
              [
                [0, "All"],
                [40, "Med+"],
                [70, "High"],
              ] as [MinIntensity, string][]
            ).map(([value, label]) => (
              <SmallToggle
                key={value}
                active={minIntensity === value}
                onClick={() => setMinIntensity(value)}
              >
                {label}
              </SmallToggle>
            ))}
          </Control>
        </div>
        {hiddenCount > 0 && (
          <p className="mt-1.5 text-[11px] text-faint">
            {hiddenCount} lower-intensity {hiddenCount === 1 ? "scare" : "scares"} hidden
          </p>
        )}

        {bulkIds.length > 0 && (
          <div className="mt-3 flex gap-2">
            <button
              onClick={() => bulkSetStatus(bulkIds, "cut")}
              className="flex-1 rounded-md bg-flare/15 px-2 py-1.5 text-xs font-medium text-flare transition-colors hover:bg-flare/25"
            >
              Cut {bulkLabel}
            </button>
            <button
              onClick={() => bulkSetStatus(bulkIds, "kept")}
              className="flex-1 rounded-md bg-seam/70 px-2 py-1.5 text-xs font-medium text-dust transition-colors hover:text-glow"
            >
              Ignore {bulkLabel}
            </button>
          </div>
        )}
          </>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {!showDetections ? null : analysisError ? (
          <p className="px-2 py-3 text-xs leading-relaxed text-dust">
            Couldn't analyze the audio, so nothing was detected automatically.
            You can still mark cuts by hand with <Kbd>I</Kbd> and <Kbd>O</Kbd>.
          </p>
        ) : analyzing ? (
          <p className="px-2 py-3 text-xs text-dust">Listening again…</p>
        ) : candidates.length === 0 ? (
          <p className="px-2 py-3 text-xs leading-relaxed text-dust">
            No sudden-loudness moments found. Try the <em>sensitive</em> setting,
            or mark cuts by hand with <Kbd>I</Kbd> and <Kbd>O</Kbd>.
          </p>
        ) : shown.length === 0 ? (
          <p className="px-2 py-3 text-xs leading-relaxed text-dust">
            All {candidates.length} detected scares are below this intensity.
            Switch the filter back to <em>All</em> to see them.
          </p>
        ) : (
          shown.map((c) => (
            <CandidateRow
              key={c.id}
              candidate={c}
              selected={selection?.kind === "candidate" && selection.id === c.id}
            />
          ))
        )}

        {manualCuts.length > 0 && (
          <>
            <p className="mt-4 mb-1 px-2 font-mono text-[10px] tracking-[0.2em] text-faint uppercase">
              Manual cuts
            </p>
            {manualCuts.map((m) => (
              <div
                key={m.id}
                className={`group mb-1 flex items-center justify-between rounded-md border px-3 py-2 ${
                  selection?.kind === "manual" && selection.id === m.id
                    ? "border-amber/60 bg-amber/10"
                    : "border-transparent hover:bg-seam/40"
                }`}
                onClick={() => jumpTo(m.start, m.end, { kind: "manual", id: m.id })}
              >
                <span className="font-mono text-[11px] text-glow">
                  {fmtTime(m.start)} – {fmtTime(m.end)}
                </span>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    removeManualCut(m.id);
                  }}
                  className="text-[11px] text-faint opacity-0 transition-opacity group-hover:opacity-100 hover:text-flare"
                >
                  Undo
                </button>
              </div>
            ))}
          </>
        )}
      </div>

      <div className="border-t border-seam px-4 py-3">
        <SummaryFooter />
      </div>
    </aside>
  );
}

function Control({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex-1">
      <p className="text-[10px] font-medium tracking-[0.15em] text-faint uppercase">
        {label}
      </p>
      <div className="mt-1 flex rounded-md border border-seam p-0.5">{children}</div>
    </div>
  );
}

function SmallToggle({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 rounded px-1 py-0.5 text-[10px] transition-colors ${
        active ? "bg-seam text-glow" : "text-faint hover:text-dust"
      }`}
    >
      {children}
    </button>
  );
}

function jumpTo(
  start: number,
  end: number,
  sel: { kind: "candidate" | "manual"; id: number },
) {
  const s = useStore.getState();
  s.select(sel);
  s.zoomToRange(start, end);
  s.seekTo(Math.max(0, start - 1.5));
}

function CandidateRow({
  candidate: c,
  selected,
}: {
  candidate: ScareCandidate;
  selected: boolean;
}) {
  const status = useStore((s) => s.candidateStatus[c.id] ?? "pending");
  const setStatus = useStore((s) => s.setCandidateStatus);
  const checked = useStore((s) => s.checkedIds.includes(c.id));
  const toggleChecked = useStore((s) => s.toggleChecked);

  return (
    <div
      className={`mb-1 cursor-pointer rounded-md border px-3 py-2 transition-colors ${
        selected
          ? "border-flare/60 bg-flare/10"
          : "border-transparent hover:bg-seam/40"
      } ${status === "kept" ? "opacity-50" : ""}`}
      onClick={() => jumpTo(c.start, c.end, { kind: "candidate", id: c.id })}
    >
      <div className="flex items-center justify-between">
        <span className="flex items-center gap-2">
          <button
            role="checkbox"
            aria-checked={checked}
            aria-label={`Select the scare at ${fmtTime(c.peakTime)}`}
            onClick={(e) => {
              e.stopPropagation();
              toggleChecked(c.id);
            }}
            className={`flex h-3.5 w-3.5 items-center justify-center rounded-sm border text-[9px] leading-none transition-colors ${
              checked
                ? "border-flare bg-flare text-well"
                : "border-faint hover:border-dust"
            }`}
          >
            {checked ? "✓" : ""}
          </button>
          <span className="font-mono text-[11px] text-glow">{fmtTime(c.peakTime)}</span>
        </span>
        <ScoreFlare score={c.score} />
      </div>
      <div className="mt-1.5 flex items-center justify-between">
        <span className="text-[11px] text-faint">{fmtSeconds(c.end - c.start)}</span>
        <div className="flex gap-1">
          <button
            onClick={(e) => {
              e.stopPropagation();
              setStatus(c.id, status === "cut" ? "pending" : "cut");
            }}
            className={`rounded px-2 py-0.5 text-[11px] font-medium transition-colors ${
              status === "cut"
                ? "bg-flare text-well"
                : "bg-seam/70 text-dust hover:text-flare"
            }`}
          >
            Cut
          </button>
          <button
            onClick={(e) => {
              e.stopPropagation();
              setStatus(c.id, status === "kept" ? "pending" : "kept");
            }}
            className={`rounded px-2 py-0.5 text-[11px] font-medium transition-colors ${
              status === "kept"
                ? "bg-dust text-well"
                : "bg-seam/70 text-dust hover:text-glow"
            }`}
          >
            Ignore
          </button>
        </div>
      </div>
    </div>
  );
}

/** Score as a tiny flame meter: hotter = taller bars. */
function ScoreFlare({ score }: { score: number }) {
  const bars = 4;
  const lit = Math.max(1, Math.round((score / 100) * bars));
  return (
    <span className="flex items-end gap-[2px]" title={`intensity ${score} of 100`}>
      {Array.from({ length: bars }, (_, i) => (
        <span
          key={i}
          className={`w-[3px] rounded-sm ${i < lit ? "bg-flare" : "bg-seam"}`}
          style={{ height: 4 + i * 2.5 }}
        />
      ))}
    </span>
  );
}

function SummaryFooter() {
  const analysis = useStore((s) => s.analysis);
  const candidateStatus = useStore((s) => s.candidateStatus);
  const manualCuts = useStore((s) => s.manualCuts);
  const showDetections = useStore((s) => s.showDetections);
  const cutCandidates = showDetections
    ? (analysis?.candidates ?? []).filter((c) => candidateStatus[c.id] === "cut")
    : [];
  const totalCut =
    cutCandidates.reduce((acc, c) => acc + (c.end - c.start), 0) +
    manualCuts.reduce((acc, m) => acc + (m.end - m.start), 0);
  const count = cutCandidates.length + manualCuts.length;
  return (
    <p className="text-xs text-dust">
      {count === 0 ? (
        "Nothing marked for removal yet."
      ) : (
        <>
          <span className="text-glow">{count}</span> cut{count === 1 ? "" : "s"} ·{" "}
          <span className="text-glow">{fmtSeconds(totalCut)}</span> removed
        </>
      )}
    </p>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-seam bg-well px-1 font-mono text-[10px] text-dust">
      {children}
    </kbd>
  );
}
