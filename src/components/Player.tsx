import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "../store";

const REVERSE_STEP_S = 0.08; // seek-step cadence for reverse shuttle
// Every playhead push repaints the timeline and re-runs a store subscription
// for each visible row, so pushing one per animation frame saturated the main
// thread and made clicks feel dead. 25/sec is smooth to the eye and leaves the
// UI responsive.
const PLAYHEAD_PUSH_MS = 40;

type SinkCapableMedia = HTMLMediaElement & {
  setSinkId?: (id: string) => Promise<void>;
};

export default function Player() {
  const proxyUrl = useStore((s) => s.proxyUrl);
  const shuttle = useStore((s) => s.shuttle);
  const seekReq = useStore((s) => s.seekReq);
  const setPlayhead = useStore((s) => s.setPlayhead);
  const videoRef = useRef<HTMLVideoElement>(null);
  const rafRef = useRef(0);
  const shuttleRef = useRef(0);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [mediaState, setMediaState] = useState("loading…");
  const [audioReport, setAudioReport] = useState("audio: not started");
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [sinkId, setSinkId] = useState("");

  // Deliberately NOT routed through Web Audio. In Safari,
  // createMediaElementSource on cross-origin media (this video is served from
  // asset.localhost, the page from tauri.localhost) silences the element
  // outright — the diagnostic would have become the bug. Safari exposes a
  // decoded-byte counter instead, which measures the same thing and touches
  // nothing.
  const audioBytesRef = useRef(0);

  // Independent of the video: an oscillator proves whether this webview can
  // make any sound at all, which splits "the app cannot play audio" from
  // "this particular file's audio never arrives".
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

  // Output device picker, when the webview supports routing at all.
  useEffect(() => {
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

  // seek requests from the timeline / panel
  useEffect(() => {
    const v = videoRef.current;
    if (v && Number.isFinite(seekReq.t)) v.currentTime = seekReq.t;
  }, [seekReq]);

  // Forward shuttle uses native playback; reverse pauses and step-seeks below.
  //
  // useLayoutEffect, not useEffect: WKWebView only allows audio to start inside
  // the user gesture that asked for it. A passive effect runs after paint, by
  // which point the gesture has expired and playback is either silent or
  // refused outright. Layout effects flush in the same task as the click.
  useLayoutEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    if (shuttle > 0) {
      v.playbackRate = Math.min(shuttle, 16);
      v.muted = false;
      v.volume = 1;
      void v
        .play()
        .then(() => setPlaybackError(null))
        .catch((error: unknown) => {
          // Do not swallow this: a silent failure here is indistinguishable
          // from a broken file.
          setPlaybackError(String(error));
          useStore.getState().setShuttle(0);
        });
    } else {
      v.pause();
      v.playbackRate = 1;
    }
  }, [shuttle]);

  // report time while playing; drive reverse shuttle with stepped seeks
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
          setPlayhead(v.currentTime);
        }
        if (now - lastReport >= 500) {
          lastReport = now;
          // audioTracks is the question that matters: does the webview
          // believe this file has audio at all? If it reports 0 while ffprobe
          // and VLC both see a stereo AAC track, the container is being
          // parsed wrong and no amount of volume will help.
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
              (decoded === undefined ? "" : ` bytes=${(decoded / 1024).toFixed(0)}KB`) +
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
    <div className="relative flex min-h-0 flex-1 items-center justify-center bg-well">
      {proxyUrl ? (
        <video
          ref={videoRef}
          src={proxyUrl}
          className="max-h-full max-w-full"
          playsInline
          preload="auto"
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
            setPlaybackError(
              `media error ${err?.code ?? "?"}: ${err?.message || "could not load the preview"}`,
            );
          }}
        />
      ) : (
        <p className="text-sm text-faint">No preview available</p>
      )}

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

      {devices.length > 0 && (
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

      {playbackError && (
        <p className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded bg-flare/20 px-3 py-1.5 text-[11px] text-glow">
          Playback was blocked: {playbackError}
        </p>
      )}
    </div>
  );
}
