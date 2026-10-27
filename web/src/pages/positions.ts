import type { Dashboard, PositionRow } from "../api";
import { money, units } from "../format";

const note = (p: PositionRow): string =>
  p.class === "mutual_fund"
    ? "cash arrives T+1/T+2 on redemption"
    : p.class === "ipo"
      ? "allotment pending · not sellable"
      : `stop ${p.stop === null ? "—" : money(p.stop)} · target ${p.target === null ? "—" : money(p.target)}`;

export function renderPositions(d: Dashboard): string {
  const rows = d.positions
    .map(
      (p) => `<tr>
  <td><span class="symbol">${p.symbol}</span> <span class="class-chip">${p.class.replace("_", " ")}</span></td>
  <td class="num right">${units(p.qty)}</td>
  <td class="num right">${money(p.avg_cost)}</td>
  <td class="num right">${money(p.qty * p.avg_cost, 0)}</td>
  <td class="${p.class === "stock" ? "" : "estimate"}">${note(p)}</td>
  <td><button class="btn" data-sell="${p.id}">Record sell</button></td>
</tr>`,
    )
    .join("");

  return `<main id="main" class="page">
  <header class="header">
    <h1>Positions</h1>
    <div class="nav">
      <a href="/">Ledger</a><a href="/positions" aria-current="page">Positions</a>
      <a href="/settings">Settings</a><a href="/calls">Audit</a>
    </div>
    <div class="double-rule"></div>
  </header>
  <section>
    <table class="ledger">
      <caption>What is held, and what it can actually be sold for</caption>
      <thead><tr>
        <th scope="col">Instrument</th><th scope="col" class="right">Qty</th>
        <th scope="col" class="right">Cost</th><th scope="col" class="right">Value at cost</th>
        <th scope="col">Execution</th><th scope="col"></th>
      </tr></thead>
      <tbody>${rows || `<tr><td colspan="6" class="empty">Nothing held.</td></tr>`}</tbody>
    </table>
  </section>
</main>`;
}
