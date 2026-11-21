export const money = (n: number, decimals = 2): string =>
  new Intl.NumberFormat("en-IN", {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(n);

export const units = (n: number): string =>
  Number.isInteger(n) ? String(n) : new Intl.NumberFormat("en-IN", { maximumFractionDigits: 3 }).format(n);

export const clock = (iso: string | null): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.valueOf())
    ? "—"
    : d.toLocaleTimeString("en-IN", { hour: "2-digit", minute: "2-digit", hour12: false });
};

export const day = (iso: string | null): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.valueOf())
    ? "—"
    : d.toLocaleDateString("en-IN", { day: "2-digit", month: "short" });
};

/** History needs the year once it is not the current one; "14 Jul" is ambiguous by March. */
export const histDate = (iso: string | null, now = new Date()): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.valueOf())) return "—";
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString("en-IN", {
    day: "2-digit",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
};

/** "09 Nov 05:34" — day and time on one line, 24-hour, for a cell that must stay one row. */
export const dayTime = (iso: string | null): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.valueOf())) return "—";
  const dPart = d.toLocaleDateString("en-IN", { day: "2-digit", month: "short" });
  const tPart = d.toLocaleTimeString("en-IN", { hour: "2-digit", minute: "2-digit", hour12: false });
  return `${dPart} ${tPart}`;
};

/**
 * An identifier shown as a label: `plan_stock_v1` → "Plan stock v1". Underscores become spaces and
 * the first letter is capitalised; the rest of the word is left alone, so `chat+search` survives.
 */
export const humanLabel = (raw: string): string =>
  titleCase(raw.replace(/_/g, " "));

/** "decide" → "Decide": a label, not an identifier. Keeps the rest of the word as it was. */
export const titleCase = (s: string): string =>
  s.length === 0 ? s : s[0].toUpperCase() + s.slice(1);

export const escapeHtml = (s: string): string =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

/** Today as a date input wants it: `YYYY-MM-DD`, local. */
export function today(): string {
  const d = new Date();
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/** `2026-11-22` → `22 Nov 2026`: the date as the board writes it. */
export function longDay(iso: string): string {
  const d = new Date(iso + "T00:00:00");
  return Number.isNaN(d.valueOf())
    ? iso
    : d.toLocaleDateString("en-IN", { day: "2-digit", month: "short", year: "numeric" });
}

/** One field accepts what a person writes: `22 Nov 2026`, or `2026-11-22`. */
export function parseDay(text: string): string | null {
  const raw = text.trim();
  if (/^\d{4}-\d{2}-\d{2}$/.test(raw)) return raw;
  const m = raw.match(/^(\d{1,2})[\s/.\-]+([A-Za-z]{3,})[\s/.\-]+(\d{4})$/);
  if (!m) return null;
  const month = MONTHS.indexOf(m[2].slice(0, 3).toLowerCase());
  const day = Number(m[1]);
  if (month < 0 || day < 1 || day > 31) return null;
  const iso = `${m[3]}-${String(month + 1).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
  const d = new Date(iso + "T00:00:00");
  return d.getMonth() === month && d.getDate() === day ? iso : null;
}

/** Floored to three decimals, the same rule the ledger applies when it costs a buy. */
export function floorUnits(n: number): number {
  return Math.floor(n * 1000) / 1000;
}

/** The most a sell may offer: what is held (floored), and never more than the field asked for. */
export function qtyLimit(side: "buy" | "sell", held: number | null): number | null {
  if (side !== "sell" || held === null) return null;
  return floorUnits(held);
}
