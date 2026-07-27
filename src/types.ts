export interface MediaTrack {
  streamIndex: number;
  kind: "audio" | "subtitle";
  codec: string;
  language: string | null;
  title: string | null;
  isDefault: boolean;
  isForced: boolean;
  isHearingImpaired: boolean;
  isVisualImpaired: boolean;
  isText: boolean;
  channels: number;
}

export interface VideoInfo {
  path: string;
  fileName: string;
  container: string;
  duration: number;
  sizeBytes: number;
  width: number;
  height: number;
  fps: number;
  videoCodec: string;
  audioCodec: string;
  audioTracks: number;
  tracks: MediaTrack[];
}

export type ContentCategory =
  | "frightening"
  | "violence"
  | "sexual"
  | "nudity"
  | "language"
  | "substances"
  | "bullying"
  | "disturbing";

export type EventAction = "review" | "cut" | "mute";

/// How far down the coarseness scale to mute spoken language.
export type ProfanityTier = "off" | "strong" | "medium" | "mild";

export interface Evidence {
  source: string;
  label: string;
  detail: string | null;
  confidence: number;
}

export interface ContentEvent {
  id: string;
  start: number;
  end: number;
  peakTime: number;
  category: ContentCategory;
  severity: 1 | 2 | 3;
  confidence: number;
  reason: string;
  suggestedAction: EventAction;
  evidence: Evidence[];
  sourceKey: string;
}

export interface AnalysisResult {
  duration: number;
  envelopeDt: number;
  envelope: number[];
  events: ContentEvent[];
}

export interface TextAnalysisResult {
  events: ContentEvent[];
  source: string;
  cueCount: number;
  warnings: string[];
}

export interface AudioEventResult {
  events: ContentEvent[];
  model: string;
  warnings: string[];
}

export interface SceneAnalysisResult {
  events: ContentEvent[];
  framesScanned: number;
  verifier: string;
  warnings: string[];
}

export interface GuideResult {
  provider: string;
  title: string | null;
  events: ContentEvent[];
  warnings: string[];
}

export interface WaveformData {
  dt: number;
  left: number[];
  right: number[];
}

export interface WaveformLevels {
  levels: { dt: number; left: Uint8Array; right: Uint8Array }[];
}

export interface ExportResult {
  outPath: string;
  keptDuration: number;
  removedDuration: number;
  mutedDuration: number;
  sizeBytes: number;
  segments: number;
}

export type EventStatus = "pending" | "cut" | "mute" | "kept";

export interface ManualCut {
  id: number;
  start: number;
  end: number;
}

export interface EditRange {
  start: number;
  end: number;
}

export interface Selection {
  kind: "event" | "manual";
  id: string | number;
}

export interface ScanState {
  running: boolean;
  pct: number;
  detail: string;
  warnings: string[];
  error: string | null;
}
