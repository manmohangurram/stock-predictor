/** Seconds as a person would say them: 900 → "15 minutes", 7200 → "2 hours". */
export function humanDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs <= 0) return "—";
  const s = Math.round(secs);
  if (s < 60) return `${s} second${s === 1 ? "" : "s"}`;
  const minutes = s / 60;
  if (minutes < 60) {
    const m = Math.round(minutes * 10) / 10;
    return `${m} minute${m === 1 ? "" : "s"}`;
  }
  const hours = Math.floor(minutes / 60);
  const rest = Math.round(minutes - hours * 60);
  if (rest === 0) return `${hours} hour${hours === 1 ? "" : "s"}`;
  return `${hours} h ${rest} min`;
}
