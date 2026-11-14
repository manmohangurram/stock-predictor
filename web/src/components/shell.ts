/**
 * Pages render their content; the chrome (rail, market state, the scrolling column) belongs to
 * `AppShell.vue`. Anything still drawn by a plain renderer returns the inner part only.
 */
export type Section = "ledger" | "audit" | "settings" | "archived";

/** Kept for the pages not yet ported: they wrap their content in this and nothing else. */
export function renderApp(_active: Section, main: string, _badges?: Partial<Record<Section, string>>): string {
  return main;
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
    <div>
      <h1>${title}</h1>
      <div class="page-sub">${subtitle}</div>
    </div>
    <div class="head-right">
    <div class="head-row">
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
    </div>
  </div>`;
}
