import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { deriveEdits, useStore } from "../store";

const REVERSE_STEP_S = 0.08; // seek-step cadence for reverse shuttle
// Every playhead push repaints the timeline and re-runs a store subscription
// for each visible row, so pushing one per animation frame saturated the main
// thread and made clicks feel dead. 25/sec is smooth to the eye and leaves the
// UI responsive.
const PLAYHEAD_PUSH_MS = 40;
const DEBUG_PLAYER = import.meta.env.DEV;

type SinkCapableMedia = HTMLMediaElement & {
  setSinkId?: (id: string) => Promise<void>;
};

export default function Player() {
  const proxyUrl = useStore((s) => s.proxyUrl);
  const rebuildingPreview = useStore((s) => s.rebuildingPreview);
  const proxyPct = useStore((s) => s.proxyPct);
  const infoPath = useStore((s) => s.info?.path);
  const shuttle = useStore((s) => s.shuttle);
  const seekReq = useStore((s) => s.seekReq);
  const skipCuts = useStore((s) => s.skipCuts);
  const setPlayhead = useStore((s) => s.setPlayhead);
  const events = useStore((s) => s.events);
  const eventStatus = useStore((s) => s.eventStatus);
  const manualCuts = useStore((s) => s.manualCuts);
  const cuts = useMemo(
    () => deriveEdits({ events, eventStatus, manualCuts }).cuts,
    [events, eventStatus, manualCuts],
  );
  const videoRef = useRef<HTMLVideoElement>(null);
  const rafRef = useRef(0);
  const shuttleRef = useRef(0);
  const cutsRef = useRef(cuts);
  const skipCutsRef = useRef(skipCuts);
  const rebuildAttemptedRef = useRef(false);
  const retriedLoadRef = useRef(false);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [mediaState, setMediaState] = useState("loading…");
  const [audioReport, setAudioReport] = useState("audio: not started");
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [sinkId, setSinkId] = useState("");
  const audioBytesRef = useRef(0);

  cutsRef.current = cuts;
  skipCutsRef.current = skipCuts;

  const skipIfInsideCut = (time: number): number => {
    if (!skipCutsRef.current) return time;
    const hit = cutsRef.current.find(
      (cut) => time >= cut.start && time < cut.end,
    );
    return hit ? hit.end : time;
  };

  const requestCompatiblePreview = () => {
    const state = useStore.getState();
    if (rebuildAttemptedRef.current || state.rebuildingPreview) return;
    rebuildAttemptedRef.current = true;
    setPlaybackError(null);
    state.setShuttle(0);
    void state.rebuildPreview().catch((error: unknown) => {
      setPlaybackError(
        `Could not build a compatible preview: ${String(error)}`,
      );
    });
  };

  const handleUnsupported = (video: HTMLVideoElement) => {
    // Fast cache hits can race the asset protocol. Reload once before
    // throwing away a preview that already played on first import.
    if (!retriedLoadRef.current) {
      retriedLoadRef.current = true;
      video.load();
      return;
    }
    requestCompatiblePreview();
  };

  useEffect(() => {
    rebuildAttemptedRef.current = false;
    retriedLoadRef.current = false;
  }, [infoPath]);

  useEffect(() => {
    retriedLoadRef.current = false;
    setPlaybackError(null);
    setMediaState("loading…");
  }, [proxyUrl]);

  const playTestTone = () => {
    try {
      const Ctor =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext })
          .webkitAudioContext;
      if (!Ctor) {
        setAudioReport("no AudioContext in this webview");
        return;
      }
      const ctx = new Ctor();
      void ctx.resume();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      gain.gain.value = 0.2;
      osc.frequency.value = 440;
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + 0.5);
      setAudioReport(`test tone sent · ctx=${ctx.state}`);
    } catch (error) {
      setAudioReport(`test tone failed: ${String(error)}`);
    }
  };

  useEffect(() => {
    shuttleRef.current = shuttle;
  }, [shuttle]);

  useEffect(() => {
    if (!DEBUG_PLAYER) {
      setDevices([]);
      return;
    }
    const supported = "setSinkId" in HTMLMediaElement.prototype;
    if (!supported) {
      setDevices([]);
      return;
    }
    void navigator.mediaDevices
      ?.enumerateDevices()
      .then((all) => setDevices(all.filter((d) => d.kind === "audiooutput")))
      .catch(() => setDevices([]));
  }, []);

  useEffect(() => {
    const v = videoRef.current as SinkCapableMedia | null;
    if (!v?.setSinkId || !sinkId) return;
    void v.setSinkId(sinkId).catch((error: unknown) => {
      setPlaybackError(`could not switch output: ${String(error)}`);
    });
  }, [sinkId]);

  useEffect(() => {
    const v = videoRef.current;
    if (!v || !Number.isFinite(seekReq.t)) return;
    v.currentTime = skipIfInsideCut(seekReq.t);
  }, [seekReq]);

  useLayoutEffect(() => {
    const v = videoRef.current;
    if (!v || rebuildingPreview) return;
    if (shuttle > 0) {
      v.playbackRate = Math.min(shuttle, 16);
      v.muted = false;
      v.volume = 1;
      void v
        .play()
        .then(() => setPlaybackError(null))
        .catch((error: unknown) => {
          const name =
            error instanceof DOMException || error instanceof Error
              ? error.name
              : "";
          if (name === "NotSupportedError") {
            handleUnsupported(v);
            return;
          }
          setPlaybackError(String(error));
          useStore.getState().setShuttle(0);
        });
    } else {
      v.pause();
      v.playbackRate = 1;
    }
  }, [shuttle, rebuildingPreview]);

  useEffect(() => {
    let last = performance.now();
    let reverseAcc = 0;
    let lastPush = 0;
    let lastReport = 0;
    const tick = (now: number) => {
      const dt = Math.min(0.25, (now - last) / 1000);
      last = now;
      const v = videoRef.current;
      if (v) {
        const rate = shuttleRef.current;
        if (rate < 0) {
          reverseAcc += dt;
          if (reverseAcc >= REVERSE_STEP_S) {
            const next = Math.max(0, v.currentTime + rate * reverseAcc);
            reverseAcc = 0;
            v.currentTime = next;
            setPlayhead(next);
            if (next <= 0) useStore.getState().setShuttle(0);
          }
        } else if (!v.paused && now - lastPush >= PLAYHEAD_PUSH_MS) {
          lastPush = now;
          const skipped = skipIfInsideCut(v.currentTime);
          if (skipped > v.currentTime + 0.04) {
            v.currentTime = skipped;
            setPlayhead(skipped);
          } else {
            setPlayhead(v.currentTime);
          }
        }
        if (DEBUG_PLAYER && now - lastReport >= 500) {
          lastReport = now;
          const media = v as HTMLVideoElement & {
            audioTracks?: { length: number };
            webkitAudioDecodedByteCount?: number;
          };
          const trackCount = media.audioTracks
            ? String(media.audioTracks.length)
            : "n/a";
          const decoded = media.webkitAudioDecodedByteCount;
          audioBytesRef.current = decoded ?? 0;
          setAudioReport(
            `audio: tracks=${trackCount} ready=${v.readyState} net=${v.networkState}` +
              ` vol=${v.volume} muted=${v.muted}` +
              (decoded === undefined
                ? ""
                : ` bytes=${(decoded / 1024).toFixed(0)}KB`) +
              (v.paused ? " (paused)" : ""),
          );
        }
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [setPlayhead]);

  return (
    <div className="relative isolate flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-well [transform:translateZ(0)]">
      {proxyUrl ? (
        <video
          key={proxyUrl}
          ref={videoRef}
          src={proxyUrl}
          className="relative z-0 h-full max-h-full w-full max-w-full object-contain"
          playsInline
          preload="metadata"
          disablePictureInPicture
          {...{ "webkit-playsinline": "true" }}
          onClick={() => {
            const s = useStore.getState();
            s.setShuttle(s.shuttle !== 0 ? 0 : 1);
          }}
          onEnded={() => useStore.getState().setShuttle(0)}
          onLoadedMetadata={(e) => {
            const v = e.currentTarget;
            setMediaState(
              `ready · ${v.videoWidth}x${v.videoHeight} · ${v.duration.toFixed(0)}s`,
            );
          }}
          onError={(e) => {
            const err = e.currentTarget.error;
            if (
              err?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED ||
              err?.code === MediaError.MEDIA_ERR_DECODE
            ) {
              handleUnsupported(e.currentTarget);
              return;
            }
            setPlaybackError(
              `media error ${err?.code ?? "?"}: ${err?.message || "could not load the preview"}`,
            );
          }}
        />
      ) : (
        <p className="text-sm text-faint">No preview available</p>
      )}

      {DEBUG_PLAYER && (
        <>
          <div className="pointer-events-none absolute top-2 left-2 rounded bg-well/70 px-2 py-1 font-mono text-[10px] text-faint">
            <span>{mediaState}</span>
            <span className="ml-2">{audioReport}</span>
          </div>
          <button
            onClick={playTestTone}
            title="Play a 440Hz beep straight from the webview, bypassing the video"
            className="absolute bottom-2 left-2 rounded border border-seam bg-well/80 px-2 py-1 text-[10px] text-dust hover:text-glow"
          >
            Test sound
          </button>
        </>
      )}

      {DEBUG_PLAYER && devices.length > 0 && (
        <select
          value={sinkId}
          onChange={(event) => setSinkId(event.target.value)}
          title="Audio output device"
          className="absolute top-2 right-2 max-w-56 rounded border border-seam bg-well/80 px-2 py-1 text-[10px] text-dust"
        >
          <option value="">System default output</option>
          {devices.map((device, index) => (
            <option key={device.deviceId} value={device.deviceId}>
              {device.label || `Output ${index + 1}`}
            </option>
          ))}
        </select>
      )}

      {rebuildingPreview && (
        <p className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded bg-well/80 px-3 py-1.5 text-[11px] text-glow">
          Building a compatible preview… {Math.round(proxyPct)}%
        </p>
      )}

      {playbackError && !rebuildingPreview && (
        <p className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded bg-flare/20 px-3 py-1.5 text-[11px] text-glow">
          {playbackError}
        </p>
      )}
    </div>
  );
}
