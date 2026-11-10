import type { ArchivedInstrument } from "../api";
import { escapeHtml, histDate, money } from "../format";
import { renderApp, renderHead } from "../components/shell";

/**
 * The record of what left the book: the name, what was done with it, and what its fills actually
 * made. There is no price left to value an archived instrument at, so the P&L comes from the fills
 * themselves — realised on the sells, and money in minus money out for the round trip.
 */
export function renderArchived(rows: ArchivedInstrument[]): string {
  const total = rows.reduce((n, r) => n + r.realised, 0);
  const body = rows.length
    ? rows
        .map((r) => {
          const tone = r.realised > 0.0005 ? "up" : r.realised < -0.0005 ? "down" : "";
          const sign = r.realised >= 0 ? "+" : "−";
          // An open round trip has nothing realised yet: say so rather than print a confident 0.
          const nothingRealised = Math.abs(r.realised) < 0.0005 && r.net_invested > 0.0005;
          const shown = r.fills === 0 || nothingRealised
            ? "—"
            : `${sign}${money(Math.abs(r.realised))}`;
          const open = Math.abs(r.net_invested) > 0.0005;
          return `<tr class="row" data-symbol="${escapeHtml(r.symbol)}" tabindex="0" role="link"
      aria-label="${escapeHtml(r.name)}, archived — ${sign}${money(Math.abs(r.realised))} realised">
  <td class="holding">
    <div class="name">${escapeHtml(r.name)}</div>
    <div class="sub">${escapeHtml(r.symbol)} · ${escapeHtml(r.class.replace("_", " "))}</div>
  </td>
  <td class="num hide-sm">${r.fills}</td>
  <td class="num hide-sm">${r.decisions}</td>
  <td class="num hide-sm">${r.last_fill ? histDate(r.last_fill) : "—"}</td>
  <td class="num ${tone}">${shown}
    <div class="sub">${
      open
        ? `${money(Math.abs(r.net_invested))} ${r.net_invested > 0 ? "in" : "out"}`
        : "closed out"
    }</div></td>
  <td class="right">
    <button class="btn ghost" type="button" data-restore="${escapeHtml(r.symbol)}"
            title="Put it back in the book — prices are fetched again on the next poll">
      Restore</button>
  </td>
</tr>`;
        })
        .join("")
    : `<tr><td colspan="6" class="empty">Nothing archived yet. Archive an instrument from its page — its record lands here.</td></tr>`;

  return renderApp(
    "archived",
    `${renderHead("Archived", "What left the book, and what it made — history without the prices", [
      { label: "Instruments", value: String(rows.length) },
      { label: "Fills kept", value: String(rows.reduce((n, r) => n + r.fills, 0)) },
      {
        label: "Realised",
        value: `${total >= 0 ? "+" : "−"}${money(Math.abs(total), 0)}`,
        tone: total >= 0 ? "pos" : "neg",
      },
    ])}
  <section class="sect">
    <table class="blotter">
      <thead><tr>
        <th scope="col">Holding</th><th scope="col" class="hide-sm">Fills</th>
        <th scope="col" class="hide-sm">Decisions</th>
        <th scope="col" class="hide-sm">Last fill</th>
        <th scope="col" class="num">Realised</th><th scope="col"></th>
      </tr></thead>
      <tbody>${body}</tbody>
    </table>
  </section>`,
  );
}
