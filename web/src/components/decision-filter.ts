/**
 * The decision-status filter, shared by the ledger and the stock overview.
 *
 * A segmented control rather than a line of links: these are mutually exclusive choices, and as
 * plain text they read like a caption. Each item carries an icon so the state is legible before the
 * word is read — and on a phone the word is dropped, not the control.
 */
export type DecisionFilter = "pending" | "approved" | "rejected" | "all";

const ITEMS: { id: DecisionFilter; label: string; hint: string; icon: string }[] = [
  {
    id: "pending",
    label: "Pending",
    hint: "Waiting for your decision",
    // an open circle: still to be decided
    icon: '<circle cx="8" cy="8" r="5.2" /><path d="M8 5.4v3l2 1.4" />',
  },
  {
    id: "approved",
    label: "Approved",
    hint: "You acted on these",
    icon: '<path d="M3.4 8.4l3 3 7-7" />',
  },
  {
    id: "rejected",
    label: "Rejected",
    hint: "You turned these down",
    icon: '<path d="M4 4l8 8M12 4l-8 8" />',
  },
  {
    id: "all",
    label: "All",
    hint: "The whole record",
    icon: '<path d="M3.5 5h9M3.5 8h9M3.5 11h6" />',
  },
];

export function isDecisionFilter(v: string | null): v is DecisionFilter {
  return v === "pending" || v === "approved" || v === "rejected" || v === "all";
}

/** `base` is the page the filter lives on, so the links work from the ledger and from a stock. */
export function decisionFilterTabs(active: DecisionFilter, base = ""): string {
  return `<div class="seg-filter" role="group" aria-label="Decision status">${ITEMS.map((it) => {
    const on = it.id === active;
    // The base may already carry a query (?mode=long), so this cannot hard-code "?".
    const href = `${base}${base.includes("?") ? "&" : "?"}decisions=${it.id}`;
    return `<a href="${href}" class="seg-item${on ? " on" : ""}"
      ${on ? 'aria-current="page"' : ""} title="${it.label} — ${it.hint}"
      aria-label="${it.label}" aria-describedby="filter-hint">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        ${it.icon}
      </svg>
    </a>`;
  }).join("")}</div>`;
}
