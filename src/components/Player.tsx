import { useEffect, useRef } from "react";
import { useStore } from "../store";

export default function Player() {
  const proxyUrl = useStore((s) => s.proxyUrl);
  const playing = useStore((s) => s.playing);
  const seekReq = useStore((s) => s.seekReq);
  const setPlayhead = useStore((s) => s.setPlayhead);
  const setPlaying = useStore((s) => s.setPlaying);
  const videoRef = useRef<HTMLVideoElement>(null);
  const rafRef = useRef(0);

  // seek requests from the timeline / panel
  useEffect(() => {
    const v = videoRef.current;
    if (v && Number.isFinite(seekReq.t)) v.currentTime = seekReq.t;
  }, [seekReq]);

  // play/pause driven by the store
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    if (playing) {
      void v.play().catch(() => setPlaying(false));
    } else {
      v.pause();
    }
  }, [playing, setPlaying]);

  // report time continuously while playing
  useEffect(() => {
    const tick = () => {
      const v = videoRef.current;
      if (v && !v.paused) setPlayhead(v.currentTime);
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
          onClick={() => setPlaying(!playing)}
          onEnded={() => setPlaying(false)}
        />
      ) : (
        <p className="text-sm text-faint">No preview available</p>
      )}
    </div>
  );
}
