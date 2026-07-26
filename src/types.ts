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
}

export interface ScareCandidate {
  id: number;
  start: number;
  end: number;
  peakTime: number;
  score: number;
  jumpLu: number;
}

export interface AnalysisResult {
  duration: number;
  envelopeDt: number;
  envelope: number[];
  candidates: ScareCandidate[];
}

export interface ExportResult {
  outPath: string;
  keptDuration: number;
  removedDuration: number;
  sizeBytes: number;
  segments: number;
}

export type CandidateStatus = "pending" | "cut" | "kept";

export interface ManualCut {
  id: number;
  start: number;
  end: number;
}

export interface Cut {
  start: number;
  end: number;
}

export interface Selection {
  kind: "candidate" | "manual";
  id: number;
}
