export function fmtTime(t: number, withTenths = false): string {
  const neg = t < 0;
  const abs = Math.max(0, Math.abs(t));
  const h = Math.floor(abs / 3600);
  const m = Math.floor((abs % 3600) / 60);
  const s = Math.floor(abs % 60);
  const tenths = Math.floor((abs % 1) * 10);
  const core =
    h > 0
      ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
      : `${m}:${String(s).padStart(2, "0")}`;
  const frac = withTenths ? `.${tenths}` : "";
  return `${neg ? "-" : ""}${core}${frac}`;
}

export function fmtBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log2(bytes) / 10));
  const v = bytes / 2 ** (10 * i);
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function fmtSeconds(t: number): string {
  if (t < 60) return `${t.toFixed(1)}s`;
  return fmtTime(t);
}
