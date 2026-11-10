export type StampKind = "buy" | "sell" | "estimate" | "blocked";

/** Stamps are literal uppercase, not CSS-uppercased: the text must exist in the markup. */
export const stamp = (kind: StampKind, label: string): string =>
  `<span class="stamp" data-stamp="${kind}">${label.toUpperCase()}</span>`;

/** The tone an action is printed in. One map, so a BUY looks the same on every page. */
export const toneOf = (action: string): StampKind =>
  action === "BUY" || action === "buy"
    ? "buy"
    : action === "SELL" || action === "sell"
      ? "sell"
      : action === "BLOCKED"
        ? "blocked"
        : "estimate";

/** An action rendered as the stamp it deserves. */
export const actionStamp = (action: string): string => stamp(toneOf(action), action);

/** A status pill — pending, approved, rejected, done, failed, running — one shape everywhere. */
export const pill = (
  tone: "ok" | "todo" | "running" | "done" | "failed" | "graphite" | "",
  label: string,
  title = "",
): string =>
  `<span class="pill ${tone}"${title ? ` title="${title}"` : ""}>${label}</span>`;

/** Approved reads as done, rejected as a to-do, pending as plain. */
export const statusPill = (status: string): string =>
  status === "approved" ? pill("ok", "approved") : status === "rejected" ? pill("todo", "rejected") : pill("", "pending");
