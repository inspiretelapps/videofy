import { create } from "zustand";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AnalysisResult,
  AudioEventResult,
  ContentCategory,
  ContentEvent,
  EditRange,
  EventStatus,
  ExportResult,
  GuideResult,
  ManualCut,
  ProfanityTier,
  ScanState,
  Selection,
  TextAnalysisResult,
  VideoInfo,
  WaveformData,
  WaveformLevels,
} from "./types";

export type Stage = "welcome" | "importing" | "editor";
export type Sensitivity = "strict" | "balanced" | "sensitive";
export type SortBy = "time" | "severity" | "confidence";
export type MinSeverity = 1 | 2 | 3;

const SENSITIVITY_VALUE: Record<Sensitivity, number> = {
  strict: 0.25,
  balanced: 0.5,
  sensitive: 0.8,
};

/// Source keys produced by the text pass — see `Cue.source` in
/// `text_analysis.rs`. Used to replace, rather than merge, text results when
/// the profanity tier changes.
/// Default seconds visible when jumping to a detection. The old behaviour
/// derived the span from the event's own length, which for a 2 s clue meant an
/// ~11 s window — too tight to see what leads into the moment.
const DEFAULT_ZOOM_SPAN = 90;
const MIN_ZOOM_SPAN = 4;

const TEXT_SOURCE_KEYS = ["subtitle", "audio-description", "transcript"];

export const ALL_CATEGORIES: ContentCategory[] = [
  "frightening",
  "violence",
  "sexual",
  "nudity",
  "language",
  "substances",
  "bullying",
  "disturbing",
];

interface ExportState {
  running: boolean;
  pct: number;
  result: ExportResult | null;
  error: string | null;
}

interface SavedProject {
  eventStatus: Record<string, EventStatus>;
  manualCuts: ManualCut[];
  nextManualId: number;
  userEvents: ContentEvent[];
  subtitlePath?: string | null;
}

interface Settings {
  dddApiKey: string;
  profanityTier: ProfanityTier;
  /// Seconds visible in the timeline. Remembered across navigation and
  /// sessions: jumping between detections should not silently rescale the
  /// view the user chose.
  zoomSpan: number;
}

interface State {
  stage: Stage;
  importError: string | null;
  info: VideoInfo | null;
  proxyUrl: string | null;
  rebuildingPreview: boolean;
  keyframes: number[];
  keyframesReady: boolean;
  keyframesError: string | null;
  analysis: AnalysisResult | null;
  rawEvents: ContentEvent[];
  events: ContentEvent[];
  userEvents: ContentEvent[];
  analysisError: string | null;
  analyzing: boolean;
  sensitivity: Sensitivity;
  waveform: WaveformLevels | null;

  proxyPct: number;
  analysisPct: number;
  waveformPct: number;
  scans: {
    text: ScanState;
    audio: ScanState;
    guide: ScanState;
  };

  eventStatus: Record<string, EventStatus>;
  manualCuts: ManualCut[];
  nextManualId: number;
  selection: Selection | null;
  pendingIn: number | null;
  sortBy: SortBy;
  minSeverity: MinSeverity;
  categories: ContentCategory[];
  checkedIds: string[];
  showDetections: boolean;
  subtitlePath: string | null;

  playhead: number;
  shuttle: number;
  seekReq: { t: number; n: number };
  view: { t0: number; t1: number };
  exporting: ExportState | null;

  settings: Settings;
  guideTitle: string;
  guideYear: number | null;
  guideOffset: number;

  openFile: (path: string, subtitlePath?: string | null) => Promise<void>;
  rebuildPreview: () => Promise<void>;
  runDeepScan: () => Promise<void>;
  reset: () => void;
  setPlayhead: (t: number) => void;
  seekTo: (t: number) => void;
  setShuttle: (rate: number) => void;
  setView: (t0: number, t1: number) => void;
  zoomToRange: (start: number, end: number) => void;
  zoomBy: (factor: number) => void;
  zoomToFit: () => void;
  select: (selection: Selection | null) => void;
  setEventStatus: (id: string, status: EventStatus) => void;
  bulkSetStatus: (ids: string[], status: EventStatus) => void;
  setSortBy: (sort: SortBy) => void;
  setMinSeverity: (severity: MinSeverity) => void;
  toggleCategory: (category: ContentCategory) => void;
  toggleChecked: (id: string) => void;
  clearChecked: () => void;
  toggleDetections: () => void;
  setPendingIn: (time: number | null) => void;
  commitOut: (time: number) => void;
  removeManualCut: (id: number) => void;
  changeSensitivity: (sensitivity: Sensitivity) => Promise<void>;
  importTimingFile: () => Promise<void>;
  attachSubtitle: () => Promise<void>;
  lookupGuide: () => Promise<void>;
  setGuideIdentity: (title: string, year: number | null) => void;
  setGuideOffset: (offset: number) => void;
  updateSettings: (settings: Partial<Settings>) => void;
  rescanText: () => Promise<void>;
  exportMovie: () => Promise<void>;
  dismissExport: () => void;
}

const idleScan = (detail: string): ScanState => ({
  running: false,
  pct: 0,
  detail,
  warnings: [],
  error: null,
});

let listenersAttached = false;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

const savedSettings = readJson<Partial<Settings>>("videofy.settings");
const storedSettings: Settings = {
  dddApiKey: savedSettings?.dddApiKey ?? "",
  profanityTier: savedSettings?.profanityTier ?? "medium",
  zoomSpan: savedSettings?.zoomSpan ?? DEFAULT_ZOOM_SPAN,
};

export const useStore = create<State>()((set, get) => ({
  stage: "welcome",
  importError: null,
  info: null,
  proxyUrl: null,
  rebuildingPreview: false,
  keyframes: [],
  keyframesReady: false,
  keyframesError: null,
  analysis: null,
  rawEvents: [],
  events: [],
  userEvents: [],
  analysisError: null,
  analyzing: false,
  sensitivity: "balanced",
  waveform: null,
  proxyPct: 0,
  analysisPct: 0,
  waveformPct: 0,
  scans: {
    text: idleScan("Waiting"),
    audio: idleScan("Waiting"),
    guide: idleScan("Optional"),
  },
  eventStatus: {},
  manualCuts: [],
  nextManualId: 1,
  selection: null,
  pendingIn: null,
  sortBy: "time",
  minSeverity: 1,
  categories: [...ALL_CATEGORIES],
  checkedIds: [],
  showDetections: true,
  subtitlePath: null,
  playhead: 0,
  shuttle: 0,
  seekReq: { t: 0, n: 0 },
  view: { t0: 0, t1: 1 },
  exporting: null,
  settings: storedSettings,
  guideTitle: "",
  guideYear: null,
  guideOffset: 0,

  openFile: async (path, selectedSubtitlePath = null) => {
    if (get().stage === "importing") return;
    attachListeners(set, get);
    set({
      stage: "importing",
      importError: null,
      proxyPct: 0,
      rebuildingPreview: false,
      analysisPct: 0,
      waveformPct: 0,
      keyframes: [],
      keyframesReady: false,
      keyframesError: null,
      rawEvents: [],
      events: [],
      userEvents: [],
      eventStatus: {},
      manualCuts: [],
      scans: {
        text: idleScan("Waiting"),
        audio: idleScan("Waiting"),
        guide: idleScan("Optional"),
      },
    });
    try {
      const info = await invoke<VideoInfo>("probe_video", { path });
      const identity = inferMovieIdentity(info.fileName);
      const saved = loadProject(info);
      const subtitlePath = selectedSubtitlePath ?? saved?.subtitlePath ?? null;
      set({
        info,
        subtitlePath,
        guideTitle: identity.title,
        guideYear: identity.year,
        eventStatus: saved?.eventStatus ?? {},
        manualCuts: saved?.manualCuts ?? [],
        nextManualId: saved?.nextManualId ?? 1,
        userEvents: saved?.userEvents ?? [],
      });
      const proxyPath = await invoke<string>("generate_proxy", {
        path,
        duration: info.duration,
        sourceHeight: info.height,
        forceTranscode: false,
      });
      const rawEvents = dedupeById([...(saved?.userEvents ?? [])]);
      const events = fuseEvents(rawEvents);
      const statuses = { ...(saved?.eventStatus ?? {}) };
      for (const event of events) statuses[event.id] ??= "pending";
      set({
        stage: "editor",
        proxyUrl: convertFileSrc(proxyPath),
        rebuildingPreview: false,
        keyframes: [],
        keyframesReady: false,
        analysis: null,
        rawEvents,
        events,
        waveform: null,
        eventStatus: statuses,
        selection: null,
        pendingIn: null,
        checkedIds: [],
        playhead: 0,
        shuttle: 0,
        view: { t0: 0, t1: info.duration },
      });

      // The preview is the only thing required to start editing. Everything
      // else fills in behind it. Running these decoders alongside preview
      // generation made the preview itself slower and Promise.all kept the
      // opening screen visible until the slowest job finished.
      void invoke<number[]>("get_keyframes", { path })
        .then((keyframes) => {
          if (get().info?.path === info.path) {
            set(
              keyframes.length > 0
                ? { keyframes, keyframesReady: true, keyframesError: null }
                : {
                    keyframes: [],
                    keyframesReady: false,
                    keyframesError:
                      "No safe video keyframes were found for lossless export.",
                  },
            );
          }
        })
        .catch((error) => {
          if (get().info?.path === info.path) {
            set({ keyframesReady: false, keyframesError: String(error) });
          }
        });

      void (async () => {
        try {
          const waveformData = await invoke<WaveformData>("get_waveform", {
            path,
            duration: info.duration,
          });
          if (get().info?.path !== info.path) return;
          set({ waveform: buildWaveformLevels(waveformData) });
        } catch {
          // The loudness envelope below remains a usable timeline fallback.
        }
        if (get().info?.path !== info.path) return;

        try {
          const analysis = await invoke<AnalysisResult>("analyze_audio", {
            path,
            duration: info.duration,
            sensitivity: SENSITIVITY_VALUE[get().sensitivity],
          });
          if (get().info?.path !== info.path) return;
          const current = get();
          const nextRaw = dedupeById([
            ...current.rawEvents.filter(
              (event) => event.sourceKey !== "loudness",
            ),
            ...analysis.events,
          ]);
          const nextEvents = fuseEvents(nextRaw);
          set({
            analysis,
            rawEvents: nextRaw,
            events: nextEvents,
            eventStatus: carryEventStatuses(
              current.events,
              nextEvents,
              current.eventStatus,
            ),
            analysisError: null,
          });
        } catch (error) {
          if (get().info?.path === info.path) {
            set({ analysisError: String(error) });
          }
        }
        if (get().info?.path === info.path) void get().runDeepScan();
      })();
    } catch (error) {
      set({ stage: "welcome", importError: String(error), info: null });
    }
  },

  runDeepScan: async () => {
    const { info } = get();
    if (!info) return;
    set((state) => ({
      scans: {
        ...state.scans,
        text: idleScan("Queued"),
        audio: idleScan("Queued"),
      },
    }));

    const begin = (key: "text" | "audio", detail: string) =>
      set((state) => ({
        scans: { ...state.scans, [key]: { ...idleScan(detail), running: true } },
      }));

    const addResult = (
      key: "text" | "audio",
      result: { events: ContentEvent[]; warnings: string[] },
      detail: string,
    ) => {
      const current = get();
      if (current.info?.path !== info.path) return;
      const rawEvents = dedupeById([...current.rawEvents, ...result.events]);
      const events = fuseEvents(rawEvents);
      const statuses = carryEventStatuses(
        current.events,
        events,
        current.eventStatus,
      );
      set((state) => ({
        rawEvents,
        events,
        eventStatus: statuses,
        scans: {
          ...state.scans,
          [key]: {
            running: false,
            pct: 100,
            detail,
            warnings: result.warnings,
            error: null,
          },
        },
      }));
      scheduleProjectSave(get);
    };
    const fail = (key: "text" | "audio", error: unknown) => {
      if (get().info?.path !== info.path) return;
      set((state) => ({
        scans: {
          ...state.scans,
          [key]: {
            running: false,
            pct: 0,
            detail: "Unavailable",
            warnings: [],
            error: String(error),
          },
        },
      }));
    };

    // One pass at a time. Run concurrently these are three simultaneous
    // full-file decodes — Whisper, ONNX inference and JPEG frame extraction —
    // competing for memory and disk bandwidth on a 16 GB machine, which made
    // all three slower and risked swapping. Sequential is faster in practice
    // and keeps the machine usable while a movie is scanning.
    const stillCurrent = () => get().info?.path === info.path;

    try {
      begin("text", "Reading subtitles and dialogue");
      const result = await invoke<TextAnalysisResult>("analyze_text", {
        path: info.path,
        duration: info.duration,
        profanityTier: get().settings.profanityTier,
        subtitlePath: get().subtitlePath,
      });
      addResult("text", result, `${result.cueCount} timed cues from ${result.source}`);
    } catch (error) {
      fail("text", error);
    }
    if (!stillCurrent()) return;

    try {
      begin("audio", "Classifying sound events");
      const result = await invoke<AudioEventResult>("analyze_audio_events", {
        path: info.path,
        duration: info.duration,
      });
      addResult("audio", result, `${result.events.length} semantic sound clues`);
    } catch (error) {
      fail("audio", error);
    }
  },

  rebuildPreview: async () => {
    const info = get().info;
    if (!info || get().rebuildingPreview) return;
    attachListeners(set, get);
    set({ rebuildingPreview: true, proxyPct: 0 });
    try {
      const proxyPath = await invoke<string>("generate_proxy", {
        path: info.path,
        duration: info.duration,
        sourceHeight: info.height,
        forceTranscode: true,
      });
      if (get().info?.path !== info.path) return;
      set({
        proxyUrl: convertFileSrc(proxyPath),
        rebuildingPreview: false,
      });
    } catch (error) {
      if (get().info?.path !== info.path) return;
      set({ rebuildingPreview: false });
      throw error;
    }
  },

  reset: () =>
    set({
      stage: "welcome",
      info: null,
      proxyUrl: null,
      rebuildingPreview: false,
      keyframes: [],
      keyframesReady: false,
      keyframesError: null,
      analysis: null,
      rawEvents: [],
      events: [],
      userEvents: [],
      analysisError: null,
      waveform: null,
      eventStatus: {},
      manualCuts: [],
      selection: null,
      pendingIn: null,
      shuttle: 0,
      exporting: null,
      subtitlePath: null,
    }),

  setPlayhead: (time) => set({ playhead: time }),
  seekTo: (time) => {
    const duration = get().info?.duration ?? 0;
    const clamped = Math.min(Math.max(0, time), duration);
    set({ playhead: clamped, seekReq: { t: clamped, n: get().seekReq.n + 1 } });
  },
  setShuttle: (rate) => set({ shuttle: rate }),
  setView: (t0, t1) => set({ view: { t0, t1 } }),
  // Centre the remembered zoom span on the region, widening only if the region
  // itself does not fit. Navigation keeps the user's zoom level.
  zoomToRange: (start, end) => {
    const duration = get().info?.duration ?? 1;
    const wanted = Math.max(get().settings.zoomSpan, (end - start) * 1.4);
    const span = Math.min(duration, wanted);
    const centre = (start + end) / 2;
    let t0 = centre - span / 2;
    let t1 = t0 + span;
    if (t0 < 0) (t1 -= t0), (t0 = 0);
    if (t1 > duration) (t0 = Math.max(0, duration - span)), (t1 = duration);
    set({ view: { t0, t1 } });
  },

  zoomBy: (factor) => {
    const duration = get().info?.duration ?? 1;
    const { view, playhead } = get();
    const current = view.t1 - view.t0;
    const span = Math.min(duration, Math.max(MIN_ZOOM_SPAN, current * factor));
    // Keep the playhead where it is on screen when it is visible, so zooming
    // does not throw away the user's place.
    const anchor =
      playhead >= view.t0 && playhead <= view.t1
        ? playhead
        : (view.t0 + view.t1) / 2;
    const ratio = (anchor - view.t0) / current;
    let t0 = anchor - ratio * span;
    let t1 = t0 + span;
    if (t0 < 0) (t1 -= t0), (t0 = 0);
    if (t1 > duration) (t0 = Math.max(0, duration - span)), (t1 = duration);
    set({ view: { t0, t1 } });
    get().updateSettings({ zoomSpan: span });
  },

  zoomToFit: () => {
    const duration = get().info?.duration ?? 1;
    set({ view: { t0: 0, t1: duration } });
    get().updateSettings({ zoomSpan: duration });
  },
  select: (selection) => set({ selection }),

  setEventStatus: (id, status) => {
    set({ eventStatus: { ...get().eventStatus, [id]: status } });
    scheduleProjectSave(get);
  },
  bulkSetStatus: (ids, status) => {
    const statuses = { ...get().eventStatus };
    for (const id of ids) statuses[id] = status;
    set({ eventStatus: statuses, checkedIds: [] });
    scheduleProjectSave(get);
  },
  setSortBy: (sortBy) => set({ sortBy }),
  setMinSeverity: (minSeverity) => set({ minSeverity }),
  toggleCategory: (category) => {
    const categories = get().categories;
    set({
      categories: categories.includes(category)
        ? categories.filter((value) => value !== category)
        : [...categories, category],
    });
  },
  toggleChecked: (id) => {
    const checked = get().checkedIds;
    set({
      checkedIds: checked.includes(id)
        ? checked.filter((value) => value !== id)
        : [...checked, id],
    });
  },
  clearChecked: () => set({ checkedIds: [] }),
  toggleDetections: () => {
    const showing = get().showDetections;
    set({
      showDetections: !showing,
      selection: showing && get().selection?.kind === "event" ? null : get().selection,
    });
  },
  setPendingIn: (time) => set({ pendingIn: time }),
  commitOut: (time) => {
    const { pendingIn, manualCuts, nextManualId } = get();
    if (pendingIn === null || time <= pendingIn + 0.05) return;
    const cut: ManualCut = { id: nextManualId, start: pendingIn, end: time };
    set({
      manualCuts: [...manualCuts, cut].sort((a, b) => a.start - b.start),
      nextManualId: nextManualId + 1,
      pendingIn: null,
      selection: { kind: "manual", id: cut.id },
    });
    scheduleProjectSave(get);
  },
  removeManualCut: (id) => {
    set({
      manualCuts: get().manualCuts.filter((cut) => cut.id !== id),
      selection:
        get().selection?.kind === "manual" && get().selection?.id === id
          ? null
          : get().selection,
    });
    scheduleProjectSave(get);
  },

  changeSensitivity: async (sensitivity) => {
    const { info } = get();
    if (!info) return;
    set({ sensitivity, analyzing: true });
    try {
      const analysis = await invoke<AnalysisResult>("analyze_audio", {
        path: info.path,
        duration: info.duration,
        sensitivity: SENSITIVITY_VALUE[sensitivity],
      });
      const current = get();
      if (current.info?.path !== info.path) return;
      const rawEvents = dedupeById([
        ...current.rawEvents.filter((event) => event.sourceKey !== "loudness"),
        ...analysis.events,
      ]);
      const events = fuseEvents(rawEvents);
      const statuses = carryEventStatuses(
        current.events,
        events,
        current.eventStatus,
      );
      set({
        analysis,
        rawEvents,
        events,
        eventStatus: statuses,
        analysisError: null,
      });
    } catch (error) {
      set({ analysisError: String(error) });
    } finally {
      set({ analyzing: false });
    }
  },

  importTimingFile: async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "Timing files", extensions: ["srt", "vtt", "skp"] }],
    });
    if (typeof path !== "string") return;
    set((state) => ({
      scans: {
        ...state.scans,
        guide: { ...idleScan("Importing timestamps"), running: true },
      },
    }));
    try {
      const result = await invoke<GuideResult>("import_timing_file", {
        path,
        offset: get().guideOffset,
      });
      const aligned =
        get().guideOffset === 0
          ? autoAlignImportedEvents(result.events, get().rawEvents)
          : { events: result.events, offset: 0 };
      addUserEvents(set, get, aligned.events);
      set((state) => ({
        scans: {
          ...state.scans,
          guide: {
            running: false,
            pct: 100,
            detail: `${aligned.events.length} events from ${result.provider}${
              aligned.offset !== 0
                ? ` · auto-synced ${aligned.offset > 0 ? "+" : ""}${aligned.offset.toFixed(1)}s`
                : ""
            }`,
            warnings: result.warnings,
            error: null,
          },
        },
      }));
    } catch (error) {
      set((state) => ({
        scans: {
          ...state.scans,
          guide: { ...idleScan("Import failed"), error: String(error) },
        },
      }));
    }
  },

  attachSubtitle: async () => {
    if (get().scans.text.running) return;
    const path = await open({
      multiple: false,
      filters: [
        {
          name: "Subtitle files",
          extensions: ["srt", "vtt", "ass", "ssa"],
        },
      ],
    });
    if (typeof path !== "string") return;
    set({ subtitlePath: path });
    scheduleProjectSave(get);
    await get().rescanText();
  },

  lookupGuide: async () => {
    const { settings, guideTitle, guideYear } = get();
    set((state) => ({
      scans: {
        ...state.scans,
        guide: { ...idleScan("Looking up title"), running: true },
      },
    }));
    try {
      const result = await invoke<GuideResult>("lookup_content_guide", {
        apiKey: settings.dddApiKey,
        title: guideTitle,
        year: guideYear,
      });
      addUserEvents(set, get, result.events);
      set((state) => ({
        scans: {
          ...state.scans,
          guide: {
            running: false,
            pct: 100,
            detail: `${result.events.length} timestamped events from ${result.provider}`,
            warnings: result.warnings,
            error: null,
          },
        },
      }));
    } catch (error) {
      set((state) => ({
        scans: {
          ...state.scans,
          guide: { ...idleScan("Guide unavailable"), error: String(error) },
        },
      }));
    }
  },
  setGuideIdentity: (guideTitle, guideYear) => set({ guideTitle, guideYear }),
  setGuideOffset: (guideOffset) => set({ guideOffset }),
  updateSettings: (partial) => {
    const previous = get().settings;
    const settings = { ...previous, ...partial };
    set({ settings });
    localStorage.setItem("videofy.settings", JSON.stringify(settings));
    // Transcripts and subtitles are cached, so re-running the text pass after a
    // tier change is cheap — it only re-derives which words become mutes.
    if (
      settings.profanityTier !== previous.profanityTier &&
      get().stage === "editor"
    ) {
      void get().rescanText();
    }
  },

  rescanText: async () => {
    const { info, settings } = get();
    if (!info || get().scans.text.running) return;
    set((state) => ({
      scans: {
        ...state.scans,
        text: { ...idleScan("Re-reading dialogue"), running: true },
      },
    }));
    try {
      const result = await invoke<TextAnalysisResult>("analyze_text", {
        path: info.path,
        duration: info.duration,
        profanityTier: settings.profanityTier,
        subtitlePath: get().subtitlePath,
      });
      if (get().info?.path !== info.path) return;
      // Replace the previous text events rather than merging, or the old
      // tier's mutes would survive alongside the new ones.
      const current = get();
      const rawEvents = dedupeById([
        ...current.rawEvents.filter(
          (event) => !TEXT_SOURCE_KEYS.includes(event.sourceKey),
        ),
        ...result.events,
      ]);
      const events = fuseEvents(rawEvents);
      const statuses = carryEventStatuses(
        current.events,
        events,
        current.eventStatus,
      );
      set((state) => ({
        rawEvents,
        events,
        eventStatus: statuses,
        scans: {
          ...state.scans,
          text: {
            running: false,
            pct: 100,
            detail: `${result.cueCount} timed cues from ${result.source}`,
            warnings: result.warnings,
            error: null,
          },
        },
      }));
      scheduleProjectSave(get);
    } catch (error) {
      set((state) => ({
        scans: {
          ...state.scans,
          text: { ...idleScan("Unavailable"), error: String(error) },
        },
      }));
    }
  },

  exportMovie: async () => {
    const { info, keyframes, keyframesReady, keyframesError } = get();
    if (!info || get().exporting?.running) return;
    if (!keyframesReady) {
      set({
        exporting: {
          running: false,
          pct: 0,
          result: null,
          error:
            keyframesError ??
            "The lossless export map is still being prepared. Please try again shortly.",
        },
      });
      return;
    }
    const { cuts, mutes } = deriveEdits(get());
    const dot = info.fileName.lastIndexOf(".");
    const stem = dot > 0 ? info.fileName.slice(0, dot) : info.fileName;
    const extension = dot > 0 ? info.fileName.slice(dot + 1) : "mp4";
    const outPath = await save({
      defaultPath: `${stem} (clean).${extension}`,
      filters: [{ name: "Video", extensions: [extension] }],
    });
    if (!outPath) return;
    set({ exporting: { running: true, pct: 0, result: null, error: null } });
    try {
      const result = await invoke<ExportResult>("export_video", {
        path: info.path,
        outPath,
        cuts,
        mutes,
        keyframes,
        duration: info.duration,
      });
      set({ exporting: { running: false, pct: 100, result, error: null } });
    } catch (error) {
      set({
        exporting: { running: false, pct: 0, result: null, error: String(error) },
      });
    }
  },
  dismissExport: () => set({ exporting: null }),
}));

function attachListeners(
  set: (partial: Partial<State> | ((state: State) => Partial<State>)) => void,
  get: () => State,
) {
  if (listenersAttached) return;
  listenersAttached = true;
  void listen<{ pct: number }>("proxy-progress", (event) =>
    set({ proxyPct: event.payload.pct }),
  );
  void listen<{ pct: number }>("analysis-progress", (event) =>
    set({ analysisPct: event.payload.pct }),
  );
  void listen<{ pct: number }>("waveform-progress", (event) =>
    set({ waveformPct: event.payload.pct }),
  );
  const scanProgress = (
    eventName: string,
    key: "text" | "audio",
  ) =>
    listen<{ pct: number }>(eventName, (event) =>
      set((state) => ({
        scans: {
          ...state.scans,
          [key]: { ...state.scans[key], pct: event.payload.pct },
        },
      })),
    );
  void scanProgress("text-analysis-progress", "text");
  void scanProgress("whisper-model-download", "text");
  void scanProgress("audio-events-progress", "audio");
  void scanProgress("audio-model-download", "audio");
  void listen<{ pct: number }>("export-progress", (event) => {
    const exporting = get().exporting;
    if (exporting?.running)
      set({ exporting: { ...exporting, pct: event.payload.pct } });
  });
}

function addUserEvents(
  set: (partial: Partial<State> | ((state: State) => Partial<State>)) => void,
  get: () => State,
  incoming: ContentEvent[],
) {
  const current = get();
  const userEvents = dedupeById([...current.userEvents, ...incoming]);
  const rawEvents = dedupeById([...current.rawEvents, ...incoming]);
  const events = fuseEvents(rawEvents);
  const eventStatus = carryEventStatuses(
    current.events,
    events,
    current.eventStatus,
  );
  set({ userEvents, rawEvents, events, eventStatus });
  scheduleProjectSave(get);
}

function buildWaveformLevels(data: WaveformData): WaveformLevels {
  const pool = (source: Uint8Array): Uint8Array => {
    const out = new Uint8Array(Math.ceil(source.length / 4));
    for (let i = 0; i < out.length; i++) {
      let max = 0;
      for (let j = i * 4; j < Math.min(source.length, i * 4 + 4); j++) {
        if (source[j] > max) max = source[j];
      }
      out[i] = max;
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
    const previous = levels[levels.length - 1];
    levels.push({
      dt: previous.dt * 4,
      left: pool(previous.left),
      right: pool(previous.right),
    });
  }
  return { levels };
}

export function deriveEdits(state: {
  events: ContentEvent[];
  eventStatus: Record<string, EventStatus>;
  manualCuts: ManualCut[];
}): { cuts: EditRange[]; mutes: EditRange[] } {
  const cuts = state.events
    .filter((event) => state.eventStatus[event.id] === "cut")
    .map(({ start, end }) => ({ start, end }));
  cuts.push(...state.manualCuts.map(({ start, end }) => ({ start, end })));
  const mutes = state.events
    .filter((event) => state.eventStatus[event.id] === "mute")
    .map(({ start, end }) => ({ start, end }));
  return { cuts: mergeRanges(cuts), mutes: mergeRanges(mutes) };
}

function mergeRanges(ranges: EditRange[]): EditRange[] {
  ranges.sort((a, b) => a.start - b.start);
  const merged: EditRange[] = [];
  for (const range of ranges) {
    const previous = merged[merged.length - 1];
    if (previous && range.start <= previous.end + 0.05)
      previous.end = Math.max(previous.end, range.end);
    else merged.push({ ...range });
  }
  return merged;
}

function dedupeById(events: ContentEvent[]): ContentEvent[] {
  const byId = new Map<string, ContentEvent>();
  for (const event of events) byId.set(event.id, event);
  return [...byId.values()].sort((a, b) => a.start - b.start);
}

function fuseEvents(events: ContentEvent[]): ContentEvent[] {
  const clusters: ContentEvent[][] = [];
  for (const event of dedupeById(events)) {
    const cluster = clusters.find((members) => {
      const differentSources = members.every(
        (member) => member.sourceKey !== event.sourceKey,
      );
      const sameCategory = members[0].category === event.category;
      const nearSameMoment = members.some(
        (member) =>
          event.start <= member.end + 0.75 &&
          event.end >= member.start - 0.75 &&
          Math.abs(event.peakTime - member.peakTime) <= 5,
      );
      const decisiveActions = new Set(
        [...members, event]
          .map((member) => member.suggestedAction)
          .filter((action) => action !== "review"),
      );
      return (
        differentSources &&
        sameCategory &&
        nearSameMoment &&
        decisiveActions.size <= 1
      );
    });
    if (cluster) cluster.push(event);
    else clusters.push([event]);
  }

  return clusters
    .map((members) => {
      if (members.length === 1) return members[0];
      const ordered = [...members].sort(
        (a, b) =>
          Number(b.suggestedAction !== "review") -
            Number(a.suggestedAction !== "review") ||
          b.confidence - a.confidence,
      );
      const primary = ordered[0];
      const decisive = members.filter(
        (member) => member.suggestedAction !== "review",
      );
      const bounds = decisive.length > 0 ? decisive : members;
      const sourceKeys = [...new Set(members.map((member) => member.sourceKey))].sort();
      const evidence = members
        .flatMap((member) => member.evidence)
        .filter(
          (item, index, all) =>
            all.findIndex(
              (candidate) =>
                candidate.source === item.source &&
                candidate.label === item.label,
            ) === index,
        );
      const maxConfidence = Math.max(
        ...members.map((member) => member.confidence),
      );
      return {
        ...primary,
        id: `fused:${members
          .map((member) => member.id)
          .sort()
          .join("+")}`,
        start: Math.min(...bounds.map((member) => member.start)),
        end: Math.max(...bounds.map((member) => member.end)),
        peakTime: primary.peakTime,
        severity: Math.max(
          ...members.map((member) => member.severity),
        ) as 1 | 2 | 3,
        confidence: Math.min(
          0.99,
          maxConfidence + 0.07 * (sourceKeys.length - 1),
        ),
        evidence,
        sourceKey: sourceKeys.join("|"),
      };
    })
    .sort((a, b) => a.start - b.start);
}

function carryEventStatuses(
  previousEvents: ContentEvent[],
  nextEvents: ContentEvent[],
  previousStatuses: Record<string, EventStatus>,
): Record<string, EventStatus> {
  const statuses = { ...previousStatuses };
  for (const event of nextEvents) {
    if (statuses[event.id]) continue;
    const sources = new Set(event.sourceKey.split("|"));
    const prior = previousEvents
      .filter(
        (candidate) =>
          candidate.category === event.category &&
          candidate.start <= event.end &&
          candidate.end >= event.start &&
          candidate.sourceKey
            .split("|")
            .some((source) => sources.has(source)),
      )
      .sort((a, b) => {
        const overlap = (candidate: ContentEvent) =>
          Math.max(
            0,
            Math.min(candidate.end, event.end) -
              Math.max(candidate.start, event.start),
          );
        return overlap(b) - overlap(a);
      })
      .find((candidate) => (statuses[candidate.id] ?? "pending") !== "pending");
    statuses[event.id] = prior ? statuses[prior.id] : "pending";
  }
  return statuses;
}

function autoAlignImportedEvents(
  incoming: ContentEvent[],
  existing: ContentEvent[],
): { events: ContentEvent[]; offset: number } {
  const anchors = existing.filter(
    (event) =>
      event.sourceKey === "loudness" && event.category === "frightening",
  );
  const imported = incoming.filter(
    (event) => event.category === "frightening",
  );
  if (anchors.length < 3 || imported.length < 3)
    return { events: incoming, offset: 0 };
  const differences = imported
    .map((event) => {
      const nearest = anchors.reduce((best, anchor) =>
        Math.abs(anchor.peakTime - event.peakTime) <
        Math.abs(best.peakTime - event.peakTime)
          ? anchor
          : best,
      );
      return nearest.peakTime - event.peakTime;
    })
    .filter((difference) => Math.abs(difference) <= 30)
    .sort((a, b) => a - b);
  if (differences.length < 3) return { events: incoming, offset: 0 };
  const median = differences[Math.floor(differences.length / 2)];
  const inliers = differences.filter(
    (difference) => Math.abs(difference - median) <= 2.5,
  );
  if (inliers.length < 3) return { events: incoming, offset: 0 };
  const offset =
    inliers.reduce((sum, difference) => sum + difference, 0) / inliers.length;
  if (Math.abs(offset) < 0.08) return { events: incoming, offset: 0 };
  return {
    offset,
    events: incoming.map((event) => ({
      ...event,
      id: `${event.id}-sync-${Math.round(offset * 10)}`,
      start: Math.max(0, event.start + offset),
      end: Math.max(0.1, event.end + offset),
      peakTime: Math.max(0, event.peakTime + offset),
      evidence: [
        ...event.evidence,
        {
          source: "automatic timeline sync",
          label: `Matched published timestamps to local audio impacts (${offset > 0 ? "+" : ""}${offset.toFixed(1)}s)`,
          detail: null,
          confidence: 0.82,
        },
      ],
    })),
  };
}

function inferMovieIdentity(fileName: string): { title: string; year: number | null } {
  const stem = fileName.replace(/\.[^.]+$/, "");
  const yearMatch = stem.match(/\b(19\d{2}|20\d{2})\b/);
  const year = yearMatch ? Number(yearMatch[1]) : null;
  const beforeYear = yearMatch ? stem.slice(0, yearMatch.index) : stem;
  const title = beforeYear
    .replace(/[._]+/g, " ")
    .replace(
      /\b(2160p|1080p|720p|bluray|webrip|web-dl|hdr|x264|x265|h264|h265)\b.*$/i,
      "",
    )
    .replace(/\s+/g, " ")
    .trim();
  return { title: title || stem, year };
}

function projectStorageKey(info: VideoInfo) {
  return `videofy.project:${info.path}:${info.sizeBytes}:${Math.round(info.duration * 10)}`;
}

function loadProject(info: VideoInfo): SavedProject | null {
  return readJson<SavedProject>(projectStorageKey(info));
}

function scheduleProjectSave(get: () => State) {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    const state = get();
    if (!state.info) return;
    const project: SavedProject = {
      eventStatus: state.eventStatus,
      manualCuts: state.manualCuts,
      nextManualId: state.nextManualId,
      userEvents: state.userEvents,
      subtitlePath: state.subtitlePath,
    };
    localStorage.setItem(projectStorageKey(state.info), JSON.stringify(project));
  }, 150);
}

function readJson<T>(key: string): T | null {
  try {
    const value = localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : null;
  } catch {
    return null;
  }
}
