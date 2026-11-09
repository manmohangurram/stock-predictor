/**
 * One chrome for every page: a narrow rail on the left, content on the right.
 *
 * The rail exists so the money is always visible and the page never wastes its left margin on a
 * nav that floats there. Below 900px it collapses into a top strip.
 */
export type Section = "ledger" | "audit" | "settings" | "archived";

/// Two groups: the daily work, then the things you configure or clean up. A new page joins a group
/// instead of extending one long list.
const LINKS: { id: Section; label: string; href: string; group: "work" | "setup" }[] = [
  { id: "ledger", label: "Ledger", href: "/", group: "work" },
  { id: "audit", label: "Audit", href: "/calls", group: "work" },
  { id: "settings", label: "Settings", href: "/settings", group: "setup" },
  { id: "archived", label: "Archived", href: "/archived", group: "setup" },
];

export function renderRail(
  active: Section,
  badges: Partial<Record<Section, string>> = {},
): string {
  const link = (l: (typeof LINKS)[number]) =>
    `<a href="${l.href}" ${l.id === active ? 'aria-current="page" class="on"' : ""}>
      <span>${l.label}</span>${badges[l.id] ? `<span class="rail-badge">${badges[l.id]}</span>` : ""}
    </a>`;
  const group = (g: "work" | "setup") =>
    LINKS.filter((l) => l.group === g).map(link).join("");
  // A sibling rule, not a wrapper: the rail lays its children out as a column.
  const items = `${group("work")}<div class="rail-sep" role="presentation"></div>${group("setup")}`;
  return `<nav class="rail" aria-label="Sections">
    <div class="rail-brand">
      <img src="/favicon.svg" width="28" height="28" alt="" aria-hidden="true" />
      <b>Stock Predictor</b>
      <span>${new Date().toLocaleDateString("en-IN", { day: "2-digit", month: "short", year: "numeric" })}</span>
    </div>
    ${items}
  </nav>`;
}

export function renderApp(
  active: Section,
  main: string,
  badges?: Partial<Record<Section, string>>,
): string {
  return `<div class="app">${renderRail(active, badges)}<main class="main">${main}</main></div>`;
}

/** Page title plus the whole-book figures that sit beside it. */
export function renderHead(
  title: string,
  subtitle: string,
  figures: {
    label: string;
    value: string;
    tone?: "pos" | "neg";
    /** A qualifier under the number — "of 13,00,000" — for a figure that is a share of something. */
    note?: string;
  }[] = [],
  /** A page-level action, right of the figures — the one place a "create" belongs. */
  action = "",
): string {
  return `<div class="page-head">
    <div><h1>${title}</h1><div class="page-sub">${subtitle}</div></div>
    <div class="head-right">
    ${
      figures.length
        ? `<div class="figures">${figures
            .map(
              (f) => `<div><span class="lbl">${f.label}</span>
            <span class="fig-line"><b class="num ${f.tone ?? ""}">${f.value}</b>${
              f.note ? `<span class="fig-note">${f.note}</span>` : ""
            }</span></div>`,
            )
            .join("")}</div>`
        : ""
    }
    ${action}
    </div>
  </div>`;
}
