import { useState } from "react";
import {
  ALL_CATEGORIES,
  deriveEdits,
  useStore,
  type MinSeverity,
  type Sensitivity,
  type SortBy,
} from "../store";
import { fmtSeconds, fmtTime } from "../lib/format";
import type {
  ContentCategory,
  ProfanityTier,
  ContentEvent,
  ScanState,
} from "../types";

const CATEGORY_LABEL: Record<ContentCategory, string> = {
  frightening: "Scary",
  violence: "Violence",
  sexual: "Sexual",
  nudity: "Nudity",
  language: "Language",
  substances: "Substances",
  bullying: "Bullying",
  disturbing: "Disturbing",
};

const CATEGORY_COLOR: Record<ContentCategory, string> = {
  frightening: "text-flare",
  violence: "text-red-400",
  sexual: "text-pink-300",
  nudity: "text-pink-300",
  language: "text-amber",
  substances: "text-emerald-300",
  bullying: "text-orange-300",
  disturbing: "text-violet-300",
};

export default function SegmentsPanel() {
  const events = useStore((state) => state.events);
  const scans = useStore((state) => state.scans);
  const analyzing = useStore((state) => state.analyzing);
  const sensitivity = useStore((state) => state.sensitivity);
  const changeSensitivity = useStore((state) => state.changeSensitivity);
  const eventStatus = useStore((state) => state.eventStatus);
  const manualCuts = useStore((state) => state.manualCuts);
  const bulkSetStatus = useStore((state) => state.bulkSetStatus);
  const removeManualCut = useStore((state) => state.removeManualCut);
  const selection = useStore((state) => state.selection);
  const sortBy = useStore((state) => state.sortBy);
  const setSortBy = useStore((state) => state.setSortBy);
  const minSeverity = useStore((state) => state.minSeverity);
  const setMinSeverity = useStore((state) => state.setMinSeverity);
  const categories = useStore((state) => state.categories);
  const toggleCategory = useStore((state) => state.toggleCategory);
  const checkedIds = useStore((state) => state.checkedIds);
  const showDetections = useStore((state) => state.showDetections);
  const toggleDetections = useStore((state) => state.toggleDetections);
  const [showSources, setShowSources] = useState(true);
  const [showGuide, setShowGuide] = useState(false);

  const shown = events
    .filter(
      (event) =>
        event.severity >= minSeverity && categories.includes(event.category),
    )
    .sort((a, b) => {
      if (sortBy === "severity")
        return b.severity - a.severity || b.confidence - a.confidence;
      if (sortBy === "confidence") return b.confidence - a.confidence;
      return a.peakTime - b.peakTime;
    });
  const shownPending = shown.filter(
    (event) => (eventStatus[event.id] ?? "pending") === "pending",
  );
  const checkedShown = checkedIds.filter((id) =>
    shown.some((event) => event.id === id),
  );
  const bulkIds =
    checkedShown.length > 0
      ? checkedShown
      : shownPending.map((event) => event.id);
  const scansRunning = Object.values(scans).some((scan) => scan.running);

  return (
    <aside className="flex w-96 shrink-0 flex-col border-l border-seam bg-bay/40">
      <div className="border-b border-seam px-4 pt-4 pb-3">
        <div className="flex items-baseline justify-between">
          <div>
            <h2 className="font-display text-sm font-semibold tracking-wide text-glow">
              Content review
            </h2>
            <p className="mt-0.5 text-[11px] text-faint">
              {events.length} clues · {shown.length} shown
              {scansRunning ? " · scanning…" : ""}
            </p>
          </div>
          <button
            onClick={toggleDetections}
            className={`rounded px-2 py-1 text-[11px] ${
              showDetections
                ? "text-faint hover:bg-seam hover:text-glow"
                : "bg-amber/15 text-amber"
            }`}
          >
            {showDetections ? "Hide marks" : "Show marks"}
          </button>
        </div>

        <button
          onClick={() => setShowSources((value) => !value)}
          className="mt-3 flex w-full items-center justify-between text-left text-[10px] font-medium tracking-[0.15em] text-faint uppercase"
        >
          Scanner coverage <span>{showSources ? "−" : "+"}</span>
        </button>
        {showSources && (
          <div className="mt-1.5 grid grid-cols-2 gap-1.5">
            <ScanChip label="Text" scan={scans.text} />
            <ScanChip label="Sound" scan={scans.audio} />
            <ScanChip label="Picture" scan={scans.vision} />
            <ScanChip label="Guide" scan={scans.guide} />
          </div>
        )}

        <div className="mt-3 flex rounded-md border border-seam p-0.5">
          {(["strict", "balanced", "sensitive"] as Sensitivity[]).map(
            (value) => (
              <button
                key={value}
                onClick={() => void changeSensitivity(value)}
                disabled={analyzing}
                title="Changes only the weak sudden-loudness detector"
                className={`flex-1 rounded px-2 py-1 text-[11px] capitalize ${
                  sensitivity === value
                    ? "bg-seam text-glow"
                    : "text-faint hover:text-dust"
                }`}
              >
                {value}
              </button>
            ),
          )}
        </div>
        <p className="mt-1 text-[10px] text-faint">
          Sensitivity affects sudden-impact clues only.
        </p>

        <div className="mt-3 flex flex-wrap gap-1">
          {ALL_CATEGORIES.map((category) => (
            <button
              key={category}
              onClick={() => toggleCategory(category)}
              className={`rounded border px-1.5 py-0.5 text-[10px] ${
                categories.includes(category)
                  ? `border-seam bg-seam/70 ${CATEGORY_COLOR[category]}`
                  : "border-transparent text-faint opacity-50"
              }`}
            >
              {CATEGORY_LABEL[category]}
            </button>
          ))}
        </div>

        <div className="mt-3 flex gap-2">
          <Control label="Sort">
            {(
              [
                ["time", "Time"],
                ["severity", "Risk"],
                ["confidence", "Conf."],
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
          <Control label="Minimum">
            {(
              [
                [1, "All"],
                [2, "Med+"],
                [3, "High"],
              ] as [MinSeverity, string][]
            ).map(([value, label]) => (
              <SmallToggle
                key={value}
                active={minSeverity === value}
                onClick={() => setMinSeverity(value)}
              >
                {label}
              </SmallToggle>
            ))}
          </Control>
        </div>

        {bulkIds.length > 0 && (
          <div className="mt-3 grid grid-cols-3 gap-1.5">
            <BulkButton onClick={() => bulkSetStatus(bulkIds, "cut")} tone="cut">
              Cut {checkedShown.length || shownPending.length}
            </BulkButton>
            <BulkButton onClick={() => bulkSetStatus(bulkIds, "mute")} tone="mute">
              Mute
            </BulkButton>
            <BulkButton onClick={() => bulkSetStatus(bulkIds, "kept")} tone="keep">
              Keep
            </BulkButton>
          </div>
        )}

        <button
          onClick={() => setShowGuide((value) => !value)}
          className="mt-3 text-[11px] text-dust hover:text-glow"
        >
          {showGuide ? "Hide guide tools" : "Import guide timestamps…"}
        </button>
        {showGuide && <GuideTools />}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {shown.length === 0 ? (
          <p className="px-2 py-3 text-xs leading-relaxed text-dust">
            {scansRunning
              ? "The semantic scanners are still working. Results will appear here as each pass completes."
              : "No content clues match these category and severity filters."}
          </p>
        ) : (
          shown.map((event) => (
            <EventRow
              key={event.id}
              event={event}
              selected={
                selection?.kind === "event" && selection.id === event.id
              }
            />
          ))
        )}

        {manualCuts.length > 0 && (
          <>
            <p className="mt-4 mb-1 px-2 font-mono text-[10px] tracking-[0.2em] text-faint uppercase">
              Manual cuts
            </p>
            {manualCuts.map((cut) => (
              <div
                key={cut.id}
                className={`group mb-1 flex items-center justify-between rounded-md border px-3 py-2 ${
                  selection?.kind === "manual" && selection.id === cut.id
                    ? "border-amber/60 bg-amber/10"
                    : "border-transparent hover:bg-seam/40"
                }`}
                onClick={() =>
                  jumpTo(cut.start, cut.end, { kind: "manual", id: cut.id })
                }
              >
                <span className="font-mono text-[11px] text-glow">
                  {fmtTime(cut.start)} – {fmtTime(cut.end)}
                </span>
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    removeManualCut(cut.id);
                  }}
                  className="text-[11px] text-faint opacity-0 group-hover:opacity-100 hover:text-flare"
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

function ScanChip({ label, scan }: { label: string; scan: ScanState }) {
  const warning = scan.error ?? scan.warnings[0];
  return (
    <div
      className={`rounded border px-2 py-1.5 ${
        scan.error
          ? "border-flare/30 bg-flare/5"
          : scan.running
            ? "border-amber/30 bg-amber/5"
            : "border-seam bg-well/40"
      }`}
      title={warning ?? scan.detail}
    >
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-medium text-dust">{label}</span>
        <span className="font-mono text-[9px] text-faint">
          {scan.running ? `${Math.floor(scan.pct)}%` : scan.error ? "!" : "✓"}
        </span>
      </div>
      <p className="mt-0.5 truncate text-[9px] text-faint">
        {scan.error ? "Unavailable" : scan.detail}
      </p>
    </div>
  );
}

const PROFANITY_HINT: Record<ProfanityTier, string> = {
  off: "No words are muted.",
  strong: "The words almost every parent removes.",
  medium: "Adds coarse-but-common words like “ass” and “damn”.",
  mild: "Adds “hell”, “crap” and casual blasphemy. Expect false positives.",
};

function GuideTools() {
  const settings = useStore((state) => state.settings);
  const updateSettings = useStore((state) => state.updateSettings);
  const guideTitle = useStore((state) => state.guideTitle);
  const guideYear = useStore((state) => state.guideYear);
  const guideOffset = useStore((state) => state.guideOffset);
  const setGuideIdentity = useStore((state) => state.setGuideIdentity);
  const setGuideOffset = useStore((state) => state.setGuideOffset);
  const lookupGuide = useStore((state) => state.lookupGuide);
  const importTimingFile = useStore((state) => state.importTimingFile);
  const runDeepScan = useStore((state) => state.runDeepScan);
  const guideScan = useStore((state) => state.scans.guide);

  return (
    <div className="mt-2 rounded-md border border-seam bg-well/45 p-2.5">
      <div className="flex gap-1.5">
        <input
          value={guideTitle}
          onChange={(event) =>
            setGuideIdentity(event.target.value, guideYear)
          }
          placeholder="Movie title"
          className="min-w-0 flex-1 rounded border border-seam bg-well px-2 py-1 text-[11px] text-glow"
        />
        <input
          value={guideYear ?? ""}
          onChange={(event) =>
            setGuideIdentity(
              guideTitle,
              event.target.value ? Number(event.target.value) : null,
            )
          }
          placeholder="Year"
          className="w-16 rounded border border-seam bg-well px-2 py-1 text-[11px] text-glow"
        />
      </div>
      <input
        type="password"
        value={settings.dddApiKey}
        onChange={(event) => updateSettings({ dddApiKey: event.target.value })}
        placeholder="Does the Dog Die? API key"
        className="mt-1.5 w-full rounded border border-seam bg-well px-2 py-1 text-[11px] text-glow"
      />
      <div className="mt-1.5 flex gap-1.5">
        <button
          disabled={!settings.dddApiKey || guideScan.running}
          onClick={() => void lookupGuide()}
          className="flex-1 rounded bg-seam px-2 py-1 text-[11px] text-dust hover:text-glow disabled:opacity-40"
        >
          Look up guide
        </button>
        <button
          onClick={() => void importTimingFile()}
          className="flex-1 rounded bg-seam px-2 py-1 text-[11px] text-dust hover:text-glow"
        >
          Import SRT / SKP
        </button>
      </div>
      <label className="mt-1.5 flex items-center gap-2 text-[10px] text-faint">
        Imported timing offset
        <input
          type="number"
          step="0.1"
          value={guideOffset}
          onChange={(event) => setGuideOffset(Number(event.target.value))}
          className="w-20 rounded border border-seam bg-well px-1.5 py-0.5 font-mono text-glow"
        />
        seconds
      </label>
      <div className="mt-2 border-t border-seam pt-2">
        <p className="text-[10px] text-faint">Mute spoken language</p>
        <div className="mt-1 flex gap-1">
          {(
            [
              ["off", "Off"],
              ["strong", "Strong"],
              ["medium", "+ Coarse"],
              ["mild", "+ Mild"],
            ] as [ProfanityTier, string][]
          ).map(([tier, label]) => (
            <button
              key={tier}
              onClick={() => updateSettings({ profanityTier: tier })}
              className={`flex-1 rounded px-1 py-1 text-[10px] ${
                settings.profanityTier === tier
                  ? "bg-glow/15 text-glow"
                  : "bg-seam text-dust hover:text-glow"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
        <p className="mt-1 text-[10px] leading-snug text-faint">
          {PROFANITY_HINT[settings.profanityTier]}
        </p>
      </div>
      <button
        onClick={() => void runDeepScan()}
        className="mt-1.5 text-[10px] text-faint hover:text-dust"
      >
        Run semantic scans again
      </button>
      {(guideScan.error || guideScan.warnings.length > 0) && (
        <p className="mt-1.5 text-[10px] leading-snug text-amber">
          {guideScan.error ?? guideScan.warnings[0]}
        </p>
      )}
    </div>
  );
}

function EventRow({
  event,
  selected,
}: {
  event: ContentEvent;
  selected: boolean;
}) {
  const status = useStore(
    (state) => state.eventStatus[event.id] ?? "pending",
  );
  const setStatus = useStore((state) => state.setEventStatus);
  const checked = useStore((state) => state.checkedIds.includes(event.id));
  const toggleChecked = useStore((state) => state.toggleChecked);
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className={`mb-1 cursor-pointer rounded-md border px-3 py-2 ${
        selected
          ? "border-flare/60 bg-flare/10"
          : "border-transparent hover:bg-seam/40"
      } ${status === "kept" ? "opacity-50" : ""}`}
      onClick={() =>
        jumpTo(event.start, event.end, { kind: "event", id: event.id })
      }
    >
      <div className="flex items-center gap-2">
        <button
          role="checkbox"
          aria-checked={checked}
          onClick={(click) => {
            click.stopPropagation();
            toggleChecked(event.id);
          }}
          className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-sm border text-[9px] ${
            checked
              ? "border-flare bg-flare text-well"
              : "border-faint hover:border-dust"
          }`}
        >
          {checked ? "✓" : ""}
        </button>
        <span className="font-mono text-[11px] text-glow">
          {fmtTime(event.peakTime)}
        </span>
        <span
          className={`rounded bg-well/70 px-1.5 py-0.5 text-[9px] font-medium ${CATEGORY_COLOR[event.category]}`}
        >
          {CATEGORY_LABEL[event.category]}
        </span>
        <SeverityDots severity={event.severity} />
        <span className="ml-auto font-mono text-[9px] text-faint">
          {Math.round(event.confidence * 100)}%
        </span>
      </div>
      <p className="mt-1.5 text-[11px] leading-snug text-dust">
        {event.reason}
      </p>
      <div className="mt-1.5 flex items-center justify-between">
        <button
          onClick={(click) => {
            click.stopPropagation();
            setExpanded((value) => !value);
          }}
          className="text-[10px] text-faint hover:text-dust"
        >
          {fmtSeconds(event.end - event.start)} · {event.evidence.length} source
          {event.evidence.length === 1 ? "" : "s"} {expanded ? "▲" : "▼"}
        </button>
        <div className="flex gap-1">
          <StatusButton
            label="Cut"
            active={status === "cut"}
            onClick={() => setStatus(event.id, status === "cut" ? "pending" : "cut")}
          />
          <StatusButton
            label="Mute"
            active={status === "mute"}
            tone="mute"
            onClick={() =>
              setStatus(event.id, status === "mute" ? "pending" : "mute")
            }
          />
          <StatusButton
            label="Keep"
            active={status === "kept"}
            tone="keep"
            onClick={() =>
              setStatus(event.id, status === "kept" ? "pending" : "kept")
            }
          />
        </div>
      </div>
      {expanded && (
        <div className="mt-2 space-y-1 border-t border-seam/70 pt-2">
          {event.evidence.map((evidence, index) => (
            <div key={`${evidence.source}-${index}`}>
              <p className="text-[10px] text-dust">{evidence.source}</p>
              <p className="select-text text-[10px] leading-snug text-faint">
                {evidence.label}
              </p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function StatusButton({
  label,
  active,
  tone = "cut",
  onClick,
}: {
  label: string;
  active: boolean;
  tone?: "cut" | "mute" | "keep";
  onClick: () => void;
}) {
  const activeClass =
    tone === "cut"
      ? "bg-flare text-well"
      : tone === "mute"
        ? "bg-amber text-well"
        : "bg-dust text-well";
  return (
    <button
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
        active ? activeClass : "bg-seam/70 text-dust hover:text-glow"
      }`}
    >
      {label}
    </button>
  );
}

function SeverityDots({ severity }: { severity: number }) {
  return (
    <span className="flex gap-0.5" title={`severity ${severity} of 3`}>
      {[1, 2, 3].map((level) => (
        <span
          key={level}
          className={`h-1.5 w-1.5 rounded-full ${
            level <= severity ? "bg-flare" : "bg-seam"
          }`}
        />
      ))}
    </span>
  );
}

function Control({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex-1">
      <p className="text-[10px] font-medium tracking-[0.15em] text-faint uppercase">
        {label}
      </p>
      <div className="mt-1 flex rounded-md border border-seam p-0.5">
        {children}
      </div>
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
      className={`flex-1 rounded px-1 py-0.5 text-[10px] ${
        active ? "bg-seam text-glow" : "text-faint hover:text-dust"
      }`}
    >
      {children}
    </button>
  );
}

function BulkButton({
  children,
  tone,
  onClick,
}: {
  children: React.ReactNode;
  tone: "cut" | "mute" | "keep";
  onClick: () => void;
}) {
  const toneClass =
    tone === "cut"
      ? "bg-flare/15 text-flare"
      : tone === "mute"
        ? "bg-amber/15 text-amber"
        : "bg-seam/70 text-dust";
  return (
    <button
      onClick={onClick}
      className={`rounded-md px-2 py-1.5 text-[11px] font-medium ${toneClass}`}
    >
      {children}
    </button>
  );
}

function jumpTo(
  start: number,
  end: number,
  selection: { kind: "event" | "manual"; id: string | number },
) {
  const state = useStore.getState();
  state.select(selection);
  state.zoomToRange(start, end);
  state.seekTo(Math.max(0, start - 1.5));
}

function SummaryFooter() {
  const events = useStore((state) => state.events);
  const eventStatus = useStore((state) => state.eventStatus);
  const manualCuts = useStore((state) => state.manualCuts);
  const edits = deriveEdits({ events, eventStatus, manualCuts });
  const removed = edits.cuts.reduce(
    (total, range) => total + range.end - range.start,
    0,
  );
  const muted = edits.mutes.reduce(
    (total, range) => total + range.end - range.start,
    0,
  );
  if (edits.cuts.length === 0 && edits.mutes.length === 0) {
    return <p className="text-xs text-dust">Nothing marked for removal yet.</p>;
  }
  return (
    <p className="text-xs text-dust">
      <span className="text-glow">{edits.cuts.length}</span> cut
      {edits.cuts.length === 1 ? "" : "s"} ·{" "}
      <span className="text-glow">{fmtSeconds(removed)}</span>
      {edits.mutes.length > 0 && (
        <>
          {" "}
          · <span className="text-amber">{edits.mutes.length}</span> mute
          {edits.mutes.length === 1 ? "" : "s"} ({fmtSeconds(muted)})
        </>
      )}
    </p>
  );
}
