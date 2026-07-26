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
      const candidates = s.analysis?.candidates ?? [];

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
          if (sel?.kind === "candidate") {
            const cur = s.candidateStatus[sel.id] ?? "pending";
            s.setCandidateStatus(sel.id, cur === "cut" ? "pending" : "cut");
          }
          break;
        }
        case "Backspace":
        case "Delete": {
          const sel = s.selection;
          if (sel?.kind === "manual") s.removeManualCut(sel.id);
          else if (sel?.kind === "candidate") s.setCandidateStatus(sel.id, "kept");
          break;
        }
        case "[":
        case "]": {
          if (candidates.length === 0) break;
          const next = e.key === "]";
          const found = next
            ? candidates.find((c) => c.peakTime > s.playhead + 0.5)
            : [...candidates].reverse().find((c) => c.peakTime < s.playhead - 0.5);
          const c = found ?? (next ? candidates[0] : candidates[candidates.length - 1]);
          s.select({ kind: "candidate", id: c.id });
          s.zoomToRange(c.start, c.end);
          s.seekTo(Math.max(0, c.start - 1.5));
          break;
        }
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
