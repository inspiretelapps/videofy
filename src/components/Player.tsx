import { useEffect, useRef } from "react";
import { useStore } from "../store";

const REVERSE_STEP_S = 0.08; // seek-step cadence for reverse shuttle

export default function Player() {
  const proxyUrl = useStore((s) => s.proxyUrl);
  const shuttle = useStore((s) => s.shuttle);
  const seekReq = useStore((s) => s.seekReq);
  const setPlayhead = useStore((s) => s.setPlayhead);
  const videoRef = useRef<HTMLVideoElement>(null);
  const rafRef = useRef(0);
  const shuttleRef = useRef(0);

  useEffect(() => {
    shuttleRef.current = shuttle;
  }, [shuttle]);

  // seek requests from the timeline / panel
  useEffect(() => {
    const v = videoRef.current;
    if (v && Number.isFinite(seekReq.t)) v.currentTime = seekReq.t;
  }, [seekReq]);

  // forward shuttle uses native playback; reverse pauses and step-seeks below
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    if (shuttle > 0) {
      v.playbackRate = Math.min(shuttle, 16);
      void v.play().catch(() => useStore.getState().setShuttle(0));
    } else {
      v.pause();
      v.playbackRate = 1;
    }
  }, [shuttle]);

  // report time while playing; drive reverse shuttle with stepped seeks
  useEffect(() => {
    let last = performance.now();
    let reverseAcc = 0;
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
        } else if (!v.paused) {
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
        />
      ) : (
        <p className="text-sm text-faint">No preview available</p>
      )}
    </div>
  );
}
