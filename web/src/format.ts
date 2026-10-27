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

export const escapeHtml = (s: string): string =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
