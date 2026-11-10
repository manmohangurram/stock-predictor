import type { ArchivedInstrument } from "../api";
import { escapeHtml, histDate } from "../format";
import { renderApp, renderHead } from "../components/shell";
import { pill } from "../components/stamp";

/** The record of what left the book: names and what was done with them, not their price history. */
export function renderArchived(rows: ArchivedInstrument[]): string {
  const body = rows.length
    ? rows
        .map(
          (r) => `<tr class="row" data-symbol="${escapeHtml(r.symbol)}" tabindex="0" role="link"
      aria-label="${escapeHtml(r.name)}, archived">
  <td class="holding">
    <div class="name">${escapeHtml(r.name)}</div>
    <div class="sub">${escapeHtml(r.symbol)} · ${escapeHtml(r.class.replace("_", " "))}</div>
  </td>
  <td class="num">${r.fills}</td>
  <td class="num">${r.decisions}</td>
  <td class="num hide-sm">${r.last_fill ? histDate(r.last_fill) : "—"}</td>
  <td class="right">${pill("todo", "archived")}</td>
</tr>`,
        )
        .join("")
    : `<tr><td colspan="5" class="empty">Nothing archived yet. Archive an instrument from its page — its record lands here.</td></tr>`;

  return renderApp(
    "archived",
    `${renderHead("Archived", "What left the book, and what was done with it — history without the prices", [
      { label: "Instruments", value: String(rows.length) },
      { label: "Fills kept", value: String(rows.reduce((n, r) => n + r.fills, 0)) },
    ])}
  <section class="sect">
    <table class="blotter">
      <thead><tr>
        <th scope="col">Holding</th><th scope="col">Fills</th><th scope="col">Decisions</th>
        <th scope="col" class="hide-sm">Last fill</th><th scope="col"></th>
      </tr></thead>
      <tbody>${body}</tbody>
    </table>
  </section>`,
  );
}
