import { create } from "zustand";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import type {
  AnalysisResult,
  CandidateStatus,
  Cut,
  ExportResult,
  ManualCut,
  Selection,
  VideoInfo,
  WaveformData,
  WaveformLevels,
} from "./types";

export type Stage = "welcome" | "importing" | "editor";

export type Sensitivity = "strict" | "balanced" | "sensitive";
const SENSITIVITY_VALUE: Record<Sensitivity, number> = {
  strict: 0.25,
  balanced: 0.5,
  sensitive: 0.8,
};

export type SortBy = "time" | "intensity";
/** Minimum score a candidate needs to stay visible in the panel. */
export type MinIntensity = 0 | 40 | 70;

interface ExportState {
  running: boolean;
  pct: number;
  result: ExportResult | null;
  error: string | null;
}

interface State {
  stage: Stage;
  importError: string | null;
  info: VideoInfo | null;
  proxyUrl: string | null;
  keyframes: number[];
  analysis: AnalysisResult | null;
  analysisError: string | null;
  analyzing: boolean;
  sensitivity: Sensitivity;
  waveform: WaveformLevels | null;

  proxyPct: number;
  analysisPct: number;
  waveformPct: number;

  candidateStatus: Record<number, CandidateStatus>;
  manualCuts: ManualCut[];
  nextManualId: number;
  selection: Selection | null;
  pendingIn: number | null;
  sortBy: SortBy;
  minIntensity: MinIntensity;
  checkedIds: number[];

  playhead: number;
  playing: boolean;
  seekReq: { t: number; n: number };
  view: { t0: number; t1: number };

  exporting: ExportState | null;

  openFile: (path: string) => Promise<void>;
  reset: () => void;
  setPlayhead: (t: number) => void;
  seekTo: (t: number) => void;
  setPlaying: (p: boolean) => void;
  setView: (t0: number, t1: number) => void;
  zoomToRange: (start: number, end: number) => void;
  select: (sel: Selection | null) => void;
  setCandidateStatus: (id: number, status: CandidateStatus) => void;
  bulkSetStatus: (ids: number[], status: CandidateStatus) => void;
  setSortBy: (s: SortBy) => void;
  setMinIntensity: (m: MinIntensity) => void;
  toggleChecked: (id: number) => void;
  clearChecked: () => void;
  setPendingIn: (t: number | null) => void;
  commitOut: (t: number) => void;
  removeManualCut: (id: number) => void;
  changeSensitivity: (s: Sensitivity) => Promise<void>;
  exportMovie: () => Promise<void>;
  dismissExport: () => void;
}

let listenersAttached = false;

export const useStore = create<State>()((set, get) => ({
  stage: "welcome",
  importError: null,
  info: null,
  proxyUrl: null,
  keyframes: [],
  analysis: null,
  analysisError: null,
  analyzing: false,
  sensitivity: "balanced",
  waveform: null,
  proxyPct: 0,
  analysisPct: 0,
  waveformPct: 0,
  candidateStatus: {},
  manualCuts: [],
  nextManualId: 1,
  selection: null,
  pendingIn: null,
  sortBy: "time",
  minIntensity: 0,
  checkedIds: [],
  playhead: 0,
  playing: false,
  seekReq: { t: 0, n: 0 },
  view: { t0: 0, t1: 1 },
  exporting: null,

  openFile: async (path: string) => {
    // guards against double-fired drop events racing two imports
    if (get().stage === "importing") return;
    if (!listenersAttached) {
      listenersAttached = true;
      void listen<{ pct: number }>("proxy-progress", (e) =>
        set({ proxyPct: e.payload.pct }),
      );
      void listen<{ pct: number }>("analysis-progress", (e) =>
        set({ analysisPct: e.payload.pct }),
      );
      void listen<{ pct: number }>("waveform-progress", (e) =>
        set({ waveformPct: e.payload.pct }),
      );
      void listen<{ pct: number }>("export-progress", (e) => {
        const ex = get().exporting;
        if (ex?.running) set({ exporting: { ...ex, pct: e.payload.pct } });
      });
    }
    set({
      stage: "importing",
      importError: null,
      proxyPct: 0,
      analysisPct: 0,
      waveformPct: 0,
    });
    try {
      const info = await invoke<VideoInfo>("probe_video", { path });
      set({ info });
      const proxyP = invoke<string>("generate_proxy", {
        path,
        duration: info.duration,
        sourceHeight: info.height,
      });
      const kfP = invoke<number[]>("get_keyframes", { path });
      const anaP = invoke<AnalysisResult>("analyze_audio", {
        path,
        duration: info.duration,
        sensitivity: SENSITIVITY_VALUE[get().sensitivity],
      }).catch((e) => {
        set({ analysisError: String(e) });
        return null;
      });
      // waveform is display-only; a failure shouldn't block the import
      const waveP = invoke<WaveformData>("get_waveform", {
        path,
        duration: info.duration,
      }).catch(() => null);
      const [proxyPath, keyframes, analysis, waveformData] = await Promise.all([
        proxyP,
        kfP,
        anaP,
        waveP,
      ]);
      const statuses: Record<number, CandidateStatus> = {};
      for (const c of analysis?.candidates ?? []) statuses[c.id] = "pending";
      set({
        stage: "editor",
        proxyUrl: convertFileSrc(proxyPath),
        keyframes,
        analysis,
        waveform: waveformData ? buildWaveformLevels(waveformData) : null,
        candidateStatus: statuses,
        manualCuts: [],
        selection: null,
        pendingIn: null,
        checkedIds: [],
        playhead: 0,
        playing: false,
        view: { t0: 0, t1: info.duration },
      });
    } catch (e) {
      set({ stage: "welcome", importError: String(e), info: null });
    }
  },

  reset: () =>
    set({
      stage: "welcome",
      info: null,
      proxyUrl: null,
      keyframes: [],
      analysis: null,
      analysisError: null,
      waveform: null,
      candidateStatus: {},
      manualCuts: [],
      selection: null,
      pendingIn: null,
      playing: false,
      exporting: null,
    }),

  setPlayhead: (t) => set({ playhead: t }),
  seekTo: (t) => {
    const d = get().info?.duration ?? 0;
    const clamped = Math.min(Math.max(0, t), d);
    set({ playhead: clamped, seekReq: { t: clamped, n: get().seekReq.n + 1 } });
  },
  setPlaying: (p) => set({ playing: p }),
  setView: (t0, t1) => set({ view: { t0, t1 } }),

  zoomToRange: (start, end) => {
    const d = get().info?.duration ?? 1;
    const span = Math.max(end - start, 6);
    const pad = span * 0.8;
    set({
      view: {
        t0: Math.max(0, start - pad),
        t1: Math.min(d, end + pad),
      },
    });
  },

  select: (sel) => set({ selection: sel }),

  setCandidateStatus: (id, status) =>
    set({ candidateStatus: { ...get().candidateStatus, [id]: status } }),

  bulkSetStatus: (ids, status) => {
    const statuses = { ...get().candidateStatus };
    for (const id of ids) statuses[id] = status;
    set({ candidateStatus: statuses, checkedIds: [] });
  },

  setSortBy: (sortBy) => set({ sortBy }),
  setMinIntensity: (minIntensity) => set({ minIntensity }),

  toggleChecked: (id) => {
    const checked = get().checkedIds;
    set({
      checkedIds: checked.includes(id)
        ? checked.filter((c) => c !== id)
        : [...checked, id],
    });
  },
  clearChecked: () => set({ checkedIds: [] }),

  setPendingIn: (t) => set({ pendingIn: t }),

  commitOut: (t) => {
    const { pendingIn, manualCuts, nextManualId } = get();
    if (pendingIn === null || t <= pendingIn + 0.05) return;
    const cut: ManualCut = { id: nextManualId, start: pendingIn, end: t };
    set({
      manualCuts: [...manualCuts, cut].sort((a, b) => a.start - b.start),
      nextManualId: nextManualId + 1,
      pendingIn: null,
      selection: { kind: "manual", id: cut.id },
    });
  },

  removeManualCut: (id) => {
    set({
      manualCuts: get().manualCuts.filter((c) => c.id !== id),
      selection:
        get().selection?.kind === "manual" && get().selection?.id === id
          ? null
          : get().selection,
    });
  },

  changeSensitivity: async (s) => {
    const { info } = get();
    if (!info) return;
    set({ sensitivity: s, analyzing: true });
    try {
      const analysis = await invoke<AnalysisResult>("analyze_audio", {
        path: info.path,
        duration: info.duration,
        sensitivity: SENSITIVITY_VALUE[s],
      });
      const statuses: Record<number, CandidateStatus> = {};
      for (const c of analysis.candidates) statuses[c.id] = "pending";
      set({
        analysis,
        candidateStatus: statuses,
        selection: null,
        checkedIds: [],
        analysisError: null,
      });
    } catch (e) {
      set({ analysisError: String(e) });
    } finally {
      set({ analyzing: false });
    }
  },

  exportMovie: async () => {
    const { info, keyframes } = get();
    if (!info || get().exporting?.running) return;
    const cuts = deriveCuts(get());
    const dot = info.fileName.lastIndexOf(".");
    const stem = dot > 0 ? info.fileName.slice(0, dot) : info.fileName;
    const ext = dot > 0 ? info.fileName.slice(dot + 1) : "mp4";
    const outPath = await save({
      defaultPath: `${stem} (clean).${ext}`,
      filters: [{ name: "Video", extensions: [ext] }],
    });
    if (!outPath) return;
    set({ exporting: { running: true, pct: 0, result: null, error: null } });
    try {
      const result = await invoke<ExportResult>("export_video", {
        path: info.path,
        outPath,
        cuts,
        keyframes,
        duration: info.duration,
      });
      set({ exporting: { running: false, pct: 100, result, error: null } });
    } catch (e) {
      set({ exporting: { running: false, pct: 0, result: null, error: String(e) } });
    }
  },

  dismissExport: () => set({ exporting: null }),
}));

/**
 * Max-pool the peak buckets into a pyramid of coarser levels (×4 each) so the
 * timeline can pick a level with only a few buckets per pixel at any zoom.
 */
function buildWaveformLevels(data: WaveformData): WaveformLevels {
  const pool = (src: Uint8Array): Uint8Array => {
    const out = new Uint8Array(Math.ceil(src.length / 4));
    for (let i = 0; i < out.length; i++) {
      let m = 0;
      for (let j = i * 4; j < Math.min(src.length, i * 4 + 4); j++) {
        if (src[j] > m) m = src[j];
      }
      out[i] = m;
    }
    return out;
  };
  const levels = [
    {
      dt: data.dt,
      left: Uint8Array.from(data.left),
      right: Uint8Array.from(data.right),
    },
  ];
  while (levels[levels.length - 1].left.length > 2048) {
    const prev = levels[levels.length - 1];
    levels.push({ dt: prev.dt * 4, left: pool(prev.left), right: pool(prev.right) });
  }
  return { levels };
}

/** Everything currently marked for removal, merged and sorted. */
export function deriveCuts(s: {
  analysis: AnalysisResult | null;
  candidateStatus: Record<number, CandidateStatus>;
  manualCuts: ManualCut[];
}): Cut[] {
  const cuts: Cut[] = [];
  for (const c of s.analysis?.candidates ?? []) {
    if (s.candidateStatus[c.id] === "cut") cuts.push({ start: c.start, end: c.end });
  }
  for (const m of s.manualCuts) cuts.push({ start: m.start, end: m.end });
  cuts.sort((a, b) => a.start - b.start);
  const merged: Cut[] = [];
  for (const c of cuts) {
    const prev = merged[merged.length - 1];
    if (prev && c.start <= prev.end + 0.05) prev.end = Math.max(prev.end, c.end);
    else merged.push({ ...c });
  }
  return merged;
}
