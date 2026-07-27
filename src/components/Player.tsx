import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "../store";

const REVERSE_STEP_S = 0.08; // seek-step cadence for reverse shuttle
// Every playhead push repaints the timeline and re-runs a store subscription
// for each visible row, so pushing one per animation frame saturated the main
// thread and made clicks feel dead. 25/sec is smooth to the eye and leaves the
// UI responsive.
const PLAYHEAD_PUSH_MS = 40;

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

  useEffect(() => {
    shuttleRef.current = shuttle;
  }, [shuttle]);

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
  // refused outright — which is what "the video plays but there is no sound"
  // looks like. Layout effects flush in the same task as the click or keypress.
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
              `ready · ${v.videoWidth}x${v.videoHeight} · ${v.duration.toFixed(0)}s · audio=${
                // Safari-only, but the whole point here is Safari.
                (v as HTMLVideoElement & { webkitAudioDecodedByteCount?: number })
                  .webkitAudioDecodedByteCount !== undefined
                  ? "decoding"
                  : "unknown"
              }`,
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
      <p className="pointer-events-none absolute top-2 left-2 rounded bg-well/70 px-2 py-1 font-mono text-[10px] text-faint">
        {mediaState}
      </p>
      {playbackError && (
        <p className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded bg-flare/20 px-3 py-1.5 text-[11px] text-glow">
          Playback was blocked: {playbackError}
        </p>
      )}
    </div>
  );
}
