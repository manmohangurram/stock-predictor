/**
 * One chrome for every page: a narrow rail on the left, content on the right.
 *
 * The rail exists so the money is always visible and the page never wastes its left margin on a
 * nav that floats there. Below 900px it collapses into a top strip.
 */
export type Section = "ledger" | "audit" | "settings";

const LINKS: { id: Section; label: string; href: string }[] = [
  { id: "ledger", label: "Ledger", href: "/" },
  { id: "audit", label: "Audit", href: "/calls" },
  { id: "settings", label: "Settings", href: "/settings" },
];

export interface RailFoot {
  label: string;
  value: string;
  note?: string;
}

export function renderRail(active: Section, foot?: RailFoot, badges: Partial<Record<Section, string>> = {}): string {
  const items = LINKS.map(
    (l) => `<a href="${l.href}" ${l.id === active ? 'aria-current="page" class="on"' : ""}>
      <span>${l.label}</span>${badges[l.id] ? `<span class="rail-badge">${badges[l.id]}</span>` : ""}
    </a>`,
  ).join("");
  const footHtml = foot
    ? `<div class="rail-foot">
        <span class="lbl">${foot.label}</span>
        <b>${foot.value}</b>
        ${foot.note ? `<span class="rail-note">${foot.note}</span>` : ""}
      </div>`
    : "";
  return `<nav class="rail" aria-label="Sections">
    <div class="rail-brand">
      <img src="/favicon.svg" width="28" height="28" alt="" aria-hidden="true" />
      <b>Stock Predictor</b>
      <span>${new Date().toLocaleDateString("en-IN", { day: "2-digit", month: "short", year: "numeric" })}</span>
    </div>
    ${items}
    ${footHtml}
  </nav>`;
}

export function renderApp(active: Section, main: string, foot?: RailFoot, badges?: Partial<Record<Section, string>>): string {
  return `<div class="app">${renderRail(active, foot, badges)}<main class="main">${main}</main></div>`;
}

/** Page title plus the whole-book figures that sit beside it. */
export function renderHead(
  title: string,
  subtitle: string,
  figures: { label: string; value: string; tone?: "pos" | "neg" }[] = [],
): string {
  return `<div class="page-head">
    <div><h1>${title}</h1><div class="page-sub">${subtitle}</div></div>
    ${
      figures.length
        ? `<div class="figures">${figures
            .map(
              (f) => `<div><span class="lbl">${f.label}</span>
            <b class="num ${f.tone ?? ""}">${f.value}</b></div>`,
            )
            .join("")}</div>`
        : ""
    }
  </div>`;
}
