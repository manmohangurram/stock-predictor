import type { Dashboard, Trade } from "../api";
import { escapeHtml, histDate, money, units } from "../format";
import { stamp } from "../components/stamp";

/** The portfolio strip: the four numbers that decide whether anything is affordable. */
export function renderStrip(d: Dashboard): string {
  const invested = d.positions.reduce((a, p) => a + (p.value ?? p.qty * p.avg_cost), 0);
  const total = invested + d.available;
  const realised = d.realised ?? 0;
  return `<header class="strip">
  <div><span class="lbl">Total value</span><b class="num total">${money(total, 0)}</b></div>
  <div><span class="lbl">Invested</span><b class="num">${money(invested, 0)}</b></div>
  <div><span class="lbl">Cash</span><b class="num">${money(d.available, 0)}</b></div>
  <div><span class="lbl">Realised</span><b class="num ${realised < 0 ? "down" : realised > 0 ? "up" : ""}">${realised > 0 ? "+" : ""}${money(realised, 0)}</b></div>
  <div class="exposure" aria-hidden="true">
    <i class="stocks" style="width:${pct(invested, total)}%"></i>
    <i class="cash" style="width:${pct(d.available, total)}%"></i>
  </div>
</header>`;
}

const pct = (part: number, whole: number): number =>
  whole <= 0 ? 0 : Math.max(0, Math.min(100, Math.round((part / whole) * 100)));

function signalFor(d: Dashboard, symbol: string) {
  return d.signals.find((s) => s.symbol === symbol);
}

/** One blotter row. Clickable: the whole row is the link to the instrument page. */
export function renderRow(d: Dashboard, p: Dashboard["positions"][number]): string {
  const s = signalFor(d, p.symbol);
  const ltp = p.ltp ?? p.avg_cost;
  const value = p.value ?? p.qty * p.avg_cost;
  const signal = s
    ? stamp(s.action === "SELL" ? "sell" : "buy", s.action)
    : `<span class="muted">—</span>`;
  return `<tr class="row" data-symbol="${escapeHtml(p.symbol)}" tabindex="0" role="link"
      aria-label="${escapeHtml(p.symbol)}, open details">
  <td class="name">${escapeHtml(p.symbol)}${p.class !== "stock" ? ` <span class="stamp" data-stamp="estimate">MF</span>` : ""}</td>
  <td class="num">${units(p.qty)}</td>
  <td class="num hide-sm">${money(p.avg_cost)}</td>
  <td class="num">${money(ltp)}</td>
  <td class="num">${money(value, 0)}</td>
  <td class="num hide-sm">${p.stop === null ? "—" : money(p.stop)}</td>
  <td>${signal}</td>
</tr>`;
}

export function renderLedger(d: Dashboard, recent: Trade[]): string {
  const rows = d.positions.map((p) => renderRow(d, p)).join("");
  const empty = `<tr><td colspan="7" class="empty">Nothing held yet. Search on the right to add your first instrument.</td></tr>`;
  const history = recent.length
    ? recent
        .slice(0, 8)
        .map(
          (t) => `<div class="hist"><span class="stamp" data-stamp="${t.side === "buy" ? "buy" : "sell"}">${t.side}</span>
        <span>${escapeHtml(t.symbol)} ${units(t.qty)} @ ${money(t.price)}</span>
        <span class="lbl">${histDate(t.traded_at)}</span></div>`,
        )
        .join("")
    : `<p class="empty">No trades recorded yet.</p>`;

  return `<div class="shell">
  ${renderStrip(d)}
  <div class="body">
    <section class="main">
      <table class="blotter">
        <caption>Holdings — select a row for the full record</caption>
        <thead><tr>
          <th scope="col">Symbol</th><th scope="col">Qty</th><th scope="col" class="hide-sm">Avg</th>
          <th scope="col">LTP</th><th scope="col">Value</th><th scope="col" class="hide-sm">Stop</th>
          <th scope="col">Signal</th>
        </tr></thead>
        <tbody>${rows || empty}</tbody>
      </table>
    </section>
    <aside class="side">
      <div class="lbl">Add instrument</div>
      <div class="search">
        <input id="search" type="search" placeholder="INFY, Wipro, 120503…" autocomplete="off" aria-label="Search stocks and mutual funds" />
      </div>
      <div id="results" class="results" role="listbox" aria-label="Search results"></div>
      <div class="lbl" style="margin-top:18px">Recent trades</div>
      <div class="history">${history}</div>
      <div class="lbl" style="margin-top:18px">This week</div>
      <div class="pair"><span>Open positions</span><span class="num">${d.positions.length}</span></div>
      <div class="pair"><span>Signals</span><span class="num">${d.signals.length}</span></div>
      <div class="pair"><span>Budget left</span><span class="num">${money(d.available, 0)}</span></div>
    </aside>
  </div>
</div>`;
}
