import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { fmtTime } from "../lib/format";
import type { CandidateStatus } from "../types";

const COLORS = {
  bg: "#100e0c",
  ridge: "rgba(147, 137, 124, 0.32)",
  crest: "rgba(147, 137, 124, 0.65)",
  flare: "#e4572e",
  amber: "#e39a2d",
  beam: "#f4efe4",
  tick: "#5c554b",
  label: "#93897c",
};

const RULER_H = 24;

export default function Timeline() {
  const info = useStore((s) => s.info);
  const analysis = useStore((s) => s.analysis);
  const candidateStatus = useStore((s) => s.candidateStatus);
  const manualCuts = useStore((s) => s.manualCuts);
  const selection = useStore((s) => s.selection);
  const pendingIn = useStore((s) => s.pendingIn);
  const playhead = useStore((s) => s.playhead);
  const view = useStore((s) => s.view);
  const seekTo = useStore((s) => s.seekTo);
  const setView = useStore((s) => s.setView);
  const select = useStore((s) => s.select);

  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [hoverT, setHoverT] = useState<number | null>(null);
  const dragRef = useRef<{ startX: number; moved: boolean } | null>(null);

  const duration = info?.duration ?? 1;

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: el.clientHeight });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const xOf = useCallback(
    (t: number) => ((t - view.t0) / (view.t1 - view.t0)) * size.w,
    [view, size.w],
  );
  const tOf = useCallback(
    (x: number) => view.t0 + (x / size.w) * (view.t1 - view.t0),
    [view, size.w],
  );

  // wheel: zoom (vertical) and pan (horizontal), non-passive to preventDefault
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const span = view.t1 - view.t0;
      if (Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
        const dt = (e.deltaX / size.w) * span;
        let t0 = view.t0 + dt;
        let t1 = view.t1 + dt;
        if (t0 < 0) (t1 -= t0), (t0 = 0);
        if (t1 > duration) (t0 -= t1 - duration), (t1 = duration);
        setView(Math.max(0, t0), Math.min(duration, t1));
      } else {
        const rect = el.getBoundingClientRect();
        const tc = tOf(e.clientX - rect.left);
        const factor = Math.exp(e.deltaY * 0.0022);
        const newSpan = Math.min(duration, Math.max(2, span * factor));
        let t0 = tc - ((tc - view.t0) / span) * newSpan;
        let t1 = t0 + newSpan;
        if (t0 < 0) (t1 -= t0), (t0 = 0);
        if (t1 > duration) (t0 = Math.max(0, duration - newSpan)), (t1 = duration);
        setView(t0, t1);
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [view, size.w, duration, setView, tOf]);

  const hitTest = useCallback(
    (t: number) => {
      for (const m of manualCuts) {
        if (t >= m.start && t <= m.end) return { kind: "manual" as const, id: m.id };
      }
      for (const c of analysis?.candidates ?? []) {
        if (t >= c.start && t <= c.end) return { kind: "candidate" as const, id: c.id };
      }
      return null;
    },
    [manualCuts, analysis],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    dragRef.current = { startX: e.clientX, moved: false };
    const rect = wrapRef.current!.getBoundingClientRect();
    seekTo(tOf(e.clientX - rect.left));
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const rect = wrapRef.current!.getBoundingClientRect();
    const t = tOf(e.clientX - rect.left);
    setHoverT(Math.max(0, Math.min(duration, t)));
    if (dragRef.current) {
      if (Math.abs(e.clientX - dragRef.current.startX) > 3) dragRef.current.moved = true;
      if (dragRef.current.moved) seekTo(t);
    }
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const drag = dragRef.current;
    dragRef.current = null;
    if (drag && !drag.moved) {
      const rect = wrapRef.current!.getBoundingClientRect();
      select(hitTest(tOf(e.clientX - rect.left)));
    }
  };

  // ---- painting ----
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.w === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.w * dpr;
    canvas.height = size.h * dpr;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const W = size.w;
    const H = size.h;
    const waveTop = RULER_H;
    const waveH = H - RULER_H;
    const waveBottom = H;

    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, H);

    // loudness ridge
    if (analysis) {
      const { envelope, envelopeDt } = analysis;
      const heightOf = (v: number) =>
        Math.max(0, Math.min(1, (v + 58) / 52)) * (waveH - 14);
      ctx.beginPath();
      ctx.moveTo(0, waveBottom);
      for (let px = 0; px <= W; px++) {
        const tA = tOf(px);
        const tB = tOf(px + 1);
        let v = -70;
        const iA = Math.max(0, Math.floor(tA / envelopeDt));
        const iB = Math.min(envelope.length - 1, Math.ceil(tB / envelopeDt));
        for (let i = iA; i <= iB; i++) v = Math.max(v, envelope[i] ?? -70);
        ctx.lineTo(px, waveBottom - heightOf(v));
      }
      ctx.lineTo(W, waveBottom);
      ctx.closePath();
      ctx.fillStyle = COLORS.ridge;
      ctx.fill();
      ctx.strokeStyle = COLORS.crest;
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // regions: scare candidates + manual cuts
    const drawRegion = (
      start: number,
      end: number,
      style: CandidateStatus | "manual",
      score: number,
      selected: boolean,
    ) => {
      const x0 = Math.max(-2, xOf(start));
      const x1 = Math.min(W + 2, xOf(end));
      if (x1 < 0 || x0 > W || x1 - x0 < 1) return;
      const w = x1 - x0;

      if (style === "pending") {
        const alpha = 0.14 + (score / 100) * 0.3;
        const grad = ctx.createLinearGradient(0, waveTop, 0, waveBottom);
        grad.addColorStop(0, `rgba(228, 87, 46, 0)`);
        grad.addColorStop(1, `rgba(228, 87, 46, ${alpha})`);
        ctx.fillStyle = grad;
        ctx.fillRect(x0, waveTop, w, waveH);
        ctx.fillStyle = `rgba(228, 87, 46, 0.9)`;
        ctx.fillRect(x0, waveBottom - 3, w, 3);
      } else if (style === "cut" || style === "manual") {
        ctx.fillStyle = "rgba(16, 14, 12, 0.55)";
        ctx.fillRect(x0, waveTop, w, waveH);
        ctx.save();
        ctx.beginPath();
        ctx.rect(x0, waveTop, w, waveH);
        ctx.clip();
        ctx.strokeStyle =
          style === "manual" ? "rgba(227, 154, 45, 0.4)" : "rgba(228, 87, 46, 0.4)";
        ctx.lineWidth = 2;
        for (let x = x0 - waveH; x < x1; x += 9) {
          ctx.beginPath();
          ctx.moveTo(x, waveBottom);
          ctx.lineTo(x + waveH, waveTop);
          ctx.stroke();
        }
        ctx.restore();
        ctx.fillStyle =
          style === "manual" ? "rgba(227, 154, 45, 0.9)" : "rgba(228, 87, 46, 0.9)";
        ctx.fillRect(x0, waveBottom - 3, w, 3);
      } else {
        // kept: whisper of what was flagged
        ctx.strokeStyle = "rgba(147, 137, 124, 0.35)";
        ctx.setLineDash([3, 4]);
        ctx.strokeRect(x0 + 0.5, waveTop + 1.5, w - 1, waveH - 3);
        ctx.setLineDash([]);
      }

      if (selected) {
        ctx.strokeStyle = style === "manual" ? COLORS.amber : COLORS.flare;
        ctx.lineWidth = 1.5;
        ctx.strokeRect(x0 + 0.75, waveTop + 0.75, w - 1.5, waveH - 1.5);
      }
    };

    for (const c of analysis?.candidates ?? []) {
      drawRegion(
        c.start,
        c.end,
        candidateStatus[c.id] ?? "pending",
        c.score,
        selection?.kind === "candidate" && selection.id === c.id,
      );
    }
    for (const m of manualCuts) {
      drawRegion(
        m.start,
        m.end,
        "manual",
        100,
        selection?.kind === "manual" && selection.id === m.id,
      );
    }

    // ruler
    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, RULER_H);
    ctx.strokeStyle = "rgba(44, 40, 35, 0.9)";
    ctx.beginPath();
    ctx.moveTo(0, RULER_H - 0.5);
    ctx.lineTo(W, RULER_H - 0.5);
    ctx.stroke();
    const span = view.t1 - view.t0;
    const steps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
    const step = steps.find((s) => (s / span) * W >= 78) ?? 3600;
    ctx.font = "9px 'Martian Mono Variable', monospace";
    ctx.fillStyle = COLORS.label;
    for (let t = Math.ceil(view.t0 / step) * step; t <= view.t1; t += step) {
      const x = xOf(t);
      ctx.strokeStyle = COLORS.tick;
      ctx.beginPath();
      ctx.moveTo(x + 0.5, RULER_H - 7);
      ctx.lineTo(x + 0.5, RULER_H);
      ctx.stroke();
      ctx.fillText(fmtTime(t), x + 4, RULER_H - 9);
    }

    // pending IN marker
    if (pendingIn !== null) {
      const x = xOf(pendingIn);
      ctx.strokeStyle = COLORS.amber;
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      ctx.moveTo(x + 0.5, waveTop);
      ctx.lineTo(x + 0.5, H);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = COLORS.amber;
      ctx.font = "8px 'Martian Mono Variable', monospace";
      ctx.fillText("IN", x + 4, waveTop + 10);
    }

    // hover ghost
    if (hoverT !== null) {
      const x = xOf(hoverT);
      ctx.strokeStyle = "rgba(236, 229, 216, 0.22)";
      ctx.beginPath();
      ctx.moveTo(x + 0.5, waveTop);
      ctx.lineTo(x + 0.5, H);
      ctx.stroke();
      ctx.font = "9px 'Martian Mono Variable', monospace";
      const label = fmtTime(hoverT, true);
      const tw = ctx.measureText(label).width;
      const lx = Math.min(Math.max(2, x + 6), W - tw - 6);
      ctx.fillStyle = "rgba(16, 14, 12, 0.85)";
      ctx.fillRect(lx - 3, 2, tw + 6, 12);
      ctx.fillStyle = "rgba(236, 229, 216, 0.75)";
      ctx.fillText(label, lx, 11);
    }

    // playhead beam
    {
      const x = xOf(playhead);
      if (x >= -2 && x <= W + 2) {
        ctx.save();
        ctx.shadowColor = "rgba(244, 239, 228, 0.6)";
        ctx.shadowBlur = 6;
        ctx.strokeStyle = COLORS.beam;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(x, RULER_H - 6);
        ctx.lineTo(x, H);
        ctx.stroke();
        ctx.restore();
        ctx.fillStyle = COLORS.beam;
        ctx.beginPath();
        ctx.moveTo(x - 4, RULER_H - 6);
        ctx.lineTo(x + 4, RULER_H - 6);
        ctx.lineTo(x, RULER_H + 1);
        ctx.closePath();
        ctx.fill();
      }
    }
  }, [
    analysis,
    candidateStatus,
    manualCuts,
    selection,
    pendingIn,
    playhead,
    view,
    size,
    hoverT,
    xOf,
    tOf,
  ]);

  return (
    <div
      ref={wrapRef}
      className="relative h-44 shrink-0 cursor-crosshair border-t border-seam bg-well"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={() => setHoverT(null)}
      onDoubleClick={() => setView(0, duration)}
    >
      <canvas ref={canvasRef} style={{ width: size.w, height: size.h }} />
    </div>
  );
}
