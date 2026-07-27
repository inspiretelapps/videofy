import { useEffect } from "react";
import { useStore } from "../store";

export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      if (s.stage !== "editor" || s.exporting) return;
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;

      const frame = 1 / (s.info?.fps || 24);
      const events = s.events;

      switch (e.key) {
        case " ":
          e.preventDefault();
          s.setShuttle(s.shuttle !== 0 ? 0 : 1);
          break;
        // JKL shuttle: repeated presses double the speed, up to 8x
        case "j":
        case "J":
          if (!e.repeat)
            s.setShuttle(s.shuttle >= 0 ? -1 : Math.max(-8, s.shuttle * 2));
          break;
        case "k":
        case "K":
          if (!e.repeat) s.setShuttle(0);
          break;
        case "l":
        case "L":
          if (!e.repeat)
            s.setShuttle(s.shuttle <= 0 ? 1 : Math.min(8, s.shuttle * 2));
          break;
        case "ArrowLeft":
          e.preventDefault();
          s.seekTo(s.playhead - (e.shiftKey ? 10 : 1));
          break;
        case "ArrowRight":
          e.preventDefault();
          s.seekTo(s.playhead + (e.shiftKey ? 10 : 1));
          break;
        case ",":
          s.setShuttle(0);
          s.seekTo(s.playhead - frame);
          break;
        case ".":
          s.setShuttle(0);
          s.seekTo(s.playhead + frame);
          break;
        case "i":
        case "I":
          s.setPendingIn(s.playhead);
          break;
        case "o":
        case "O":
          s.commitOut(s.playhead);
          break;
        case "Escape":
          if (s.pendingIn !== null) s.setPendingIn(null);
          else s.select(null);
          break;
        case "Enter": {
          const sel = s.selection;
          if (sel?.kind === "event") {
            const id = String(sel.id);
            const current = s.eventStatus[id] ?? "pending";
            s.setEventStatus(id, current === "cut" ? "pending" : "cut");
          }
          break;
        }
        case "Backspace":
        case "Delete": {
          const sel = s.selection;
          if (sel?.kind === "manual") s.removeManualCut(Number(sel.id));
          else if (sel?.kind === "event") s.setEventStatus(String(sel.id), "kept");
          break;
        }
        case "[":
        case "]": {
          // walk every visible cut region in time order: detections (when
          // shown and above the intensity filter) plus manual cuts.
          // Selection-relative stepping, NOT playhead-relative — jumping to a
          // region parks the playhead before its start, which would make
          // "next" match the same region forever.
          const items = [
            ...(s.showDetections
              ? events
                  .filter(
                    (event) =>
                      event.severity >= s.minSeverity &&
                      s.categories.includes(event.category),
                  )
                  .map((event) => ({
                    kind: "event" as const,
                    id: event.id,
                    start: event.start,
                    end: event.end,
                  }))
              : []),
            ...s.manualCuts.map((m) => ({
              kind: "manual" as const,
              id: m.id,
              start: m.start,
              end: m.end,
            })),
          ].sort((a, b) => a.start - b.start);
          if (items.length === 0) break;
          const next = e.key === "]";
          const sel = s.selection;
          const curIdx = sel
            ? items.findIndex((i) => i.kind === sel.kind && i.id === sel.id)
            : -1;
          const target =
            curIdx >= 0
              ? items[(curIdx + (next ? 1 : -1) + items.length) % items.length]
              : next
                ? (items.find((i) => i.start > s.playhead + 0.05) ?? items[0])
                : ([...items].reverse().find((i) => i.start < s.playhead - 0.05) ??
                  items[items.length - 1]);
          s.select({ kind: target.kind, id: target.id });
          s.zoomToRange(target.start, target.end);
          s.seekTo(Math.max(0, target.start - 1.5));
          break;
        }
        case "+":
        case "=":
          e.preventDefault();
          s.zoomBy(0.5);
          break;
        case "-":
        case "_":
          e.preventDefault();
          s.zoomBy(2);
          break;
        case "0":
          e.preventDefault();
          s.zoomToFit();
          break;
        case "e":
        case "E":
          void s.exportMovie();
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
