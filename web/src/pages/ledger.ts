import type { Dashboard, Mode, Trade } from "../api";
import { renderApp, renderHead } from "../components/shell";
import { renderDecisionActions } from "../components/decision-actions";
import { decisionFilterTabs, isDecisionFilter, type DecisionFilter } from "../components/decision-filter";
import { escapeHtml, histDate, money, units } from "../format";
import { renderConfidence } from "../components/confidence";
import { stamp } from "../components/stamp";

/** The portfolio strip: the four numbers that decide whether anything is affordable. */
export function renderModeTabs(mode: Mode): string {
  const tab = (m: Mode, label: string) =>
    `<a class="mode-tab" href="/?mode=${m}" ${m === mode ? 'aria-current="page"' : ""}>${label}</a>`;
  return `<nav class="modes" aria-label="Strategy">
    ${tab("long", "Long-term")}${tab("trade", "Short-term")}
  </nav>`;
}

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

/** One holdings row. Name first: a ticker tells you nothing about what you own. */
export function renderRow(d: Dashboard, p: Dashboard["positions"][number]): string {
  const s = signalFor(d, p.symbol);
  const live = p.ltp !== null;
  const ltp = p.ltp ?? p.avg_cost;
  const invested = p.qty * p.avg_cost;
  const value = p.value ?? invested;
  const pnl = value - invested;
  const pnlPct = invested > 0 ? (pnl / invested) * 100 : 0;
  const nowCell = live ? money(ltp) : `<span class="muted">—</span>`;
  const valueCell = live ? money(value) : `<span class="muted">—</span>`;
  const pnlCell = live
    ? `${pnl >= 0 ? "+" : "−"}${money(Math.abs(pnl))}<div class="sub">${pnlPct >= 0 ? "+" : "−"}${Math.abs(pnlPct).toFixed(1)}%</div>`
    : `<span class="muted">no price yet</span>`;
  const isFund = p.class !== "stock";
  const plan =
    isFund || p.stop === null
      ? `<span class="muted">${isFund ? "units at NAV" : "—"}</span>`
      : `<span class="num">${money(p.stop)} → ${p.target === null ? "—" : money(p.target)}</span>`;
  const signal = s
    ? `<span class="signal-cell">${stamp(s.action === "SELL" ? "sell" : "buy", s.action)}${renderConfidence(s.confidence)}</span>`
    : `<span class="muted">—</span>`;
  return `<tr class="row" data-symbol="${escapeHtml(p.symbol)}" tabindex="0" role="link"
      aria-label="${escapeHtml(p.name)}, open details">
  <td class="holding" title="${escapeHtml(p.name)} · ${escapeHtml(p.symbol)}">
    <div class="name">${escapeHtml(p.name)}</div>
    <div class="sub">${escapeHtml(p.symbol)}</div></td>
  <td class="num hide-sm">${units(p.qty)}</td>
  <td class="num hide-sm">${money(p.avg_cost)}</td>
  <td class="num hide-sm">${money(invested)}</td>
  <td class="num">${nowCell}</td>
  <td class="num hide-sm">${valueCell}</td>
  <td class="num ${live ? (pnl >= 0 ? "up" : "down") : ""}">${pnlCell}</td>
  <td class="signal-col">${signal}</td>
  <td class="right hide-sm">${plan}</td>
</tr>`;
}

function section(title: string, note: string, head: string, body: string, total?: string): string {
  return `<section class="sect">
    <div class="sect-head"><h2>${title}</h2><span class="sub">${note}</span></div>
    <div class="table-wrap"><table class="blotter">
      <thead>${head}</thead>
      <tbody>${body}${total ?? ""}</tbody>
    </table></div>
  </section>`;
}

const STOCK_HEAD = `<tr><th scope="col">Holding</th><th scope="col" class="hide-sm">Qty</th>
  <th scope="col" class="hide-sm">Bought</th><th scope="col" class="hide-sm">Invested</th>
  <th scope="col">Now</th><th scope="col" class="hide-sm">Value</th>
  <th scope="col">P&amp;L</th><th scope="col">Signal</th><th scope="col" class="hide-sm">Exit plan</th></tr>`;
const FUND_HEAD = `<tr><th scope="col">Scheme</th><th scope="col" class="hide-sm">Units</th>
  <th scope="col" class="hide-sm">Avg NAV</th><th scope="col" class="hide-sm">Invested</th>
  <th scope="col">NAV today</th><th scope="col" class="hide-sm">Value</th>
  <th scope="col">P&amp;L</th><th scope="col">Signal</th><th scope="col" class="hide-sm">Execution</th></tr>`;

const subtotal = (label: string, invested: number, value: number): string => {
  const pnl = value - invested;
  return `<tr class="subtotal"><td>${label}</td><td class="hide-sm">—</td><td class="hide-sm">—</td>
    <td class="num hide-sm">${money(invested)}</td><td>—</td>
    <td class="num hide-sm">${money(value)}</td>
    <td class="num ${pnl >= 0 ? "up" : "down"}">${pnl >= 0 ? "+" : "−"}${money(Math.abs(pnl))}</td>
    <td>—</td><td class="sub hide-sm">—</td></tr>`;
};

/** One decision: what it says, and the two things you can do about it. */
function decisionRow(s: Dashboard["signals"][number]): string {
  const tone = s.action === "SELL" ? "sell" : s.action === "BUY" ? "buy" : "blocked";
  const status =
    s.status === "pending"
      ? ""
      : `<span class="pill ${s.status === "approved" ? "ok" : "todo"}">${s.status}</span>`;
  const actions = renderDecisionActions(s);
  return `<div class="feed-row decision-feed">
    <div class="decision-main">
      <div><span class="stamp" data-stamp="${tone}">${s.action}</span>
        <b title="${escapeHtml(s.symbol)}">${escapeHtml(s.symbol)}</b>
        ${s.entry === null ? "" : `<span class="num">${money(s.entry)}</span>`}
        ${s.qty === null ? "" : `<span class="lbl">${units(s.qty)} units</span>`}
        ${status}
        <span class="sub">${histDate(s.created_at)}</span></div>
      ${s.rationale ? `<p class="decision-prose">${escapeHtml(s.rationale)}</p>` : ""}
      ${
        s.action === "BLOCKED" && s.reasons.length
          ? `<p class="decision-note">${s.reasons.map((r) => escapeHtml(r)).join(" · ")}</p>`
          : ""
      }
      ${actions}
    </div>
  </div>`;
}

const sum = (rows: Dashboard["positions"]): { invested: number; value: number } => ({
  invested: rows.reduce((a, p) => a + p.qty * p.avg_cost, 0),
  value: rows.reduce((a, p) => a + (p.value ?? p.qty * p.avg_cost), 0),
});

export function renderLedger(d: Dashboard, recent: Trade[]): string {
  const inMode = recent.filter((t) => t.mode === d.mode);
  const stocks = d.positions.filter((p) => p.class === "stock");
  const funds = d.positions.filter((p) => p.class !== "stock");
  const all = sum(d.positions);
  const pnl = all.value - all.invested;

  const history = inMode.length
    ? inMode
        .slice(0, 8)
        .map(
          (t) => `<div class="feed-row"><span><span class="stamp" data-stamp="${t.side === "buy" ? "buy" : "sell"}">${t.side}</span>
        ${escapeHtml(t.symbol)} ${units(t.qty)} @ ${money(t.price)}</span>
        <span class="sub">${histDate(t.traded_at)}</span></div>`,
        )
        .join("")
    : `<p class="empty">No trades recorded yet.</p>`;

  // Rendered on the server of truth (the browser) and in tests: no window, no filter.
  const raw =
    typeof location !== "undefined"
      ? new URLSearchParams(location.search).get("decisions")
      : null;
  const filter: DecisionFilter = isDecisionFilter(raw) ? raw : "pending";
  // Blocked proposals sit with the decisions: they are what the app wanted to do and could not.
  const shown = [...d.signals, ...d.blocked].sort((a, b) => b.id - a.id);
  const advice = shown.length
    ? shown
        .slice(0, 10)
        .map((s) => decisionRow(s))
        .join("")
    : `<p class="empty">${
        filter === "pending" ? "Nothing under review." : "Nothing in this filter."
      }</p>`;

  const stocksSection = section(
    "Stocks",
    `${stocks.length} · exit plan from the last signal`,
    STOCK_HEAD,
    stocks.length
      ? stocks.map((p) => renderRow(d, p)).join("")
      : `<tr><td colspan="9" class="empty">No stocks yet — search on the right.</td></tr>`,
    stocks.length
      ? subtotal("Stocks", sum(stocks).invested, sum(stocks).value)
      : "",
  );

  const fundsSection = section(
    "Mutual funds",
    `${funds.length} · bought at NAV, no stop or target`,
    FUND_HEAD,
    funds.length
      ? funds.map((p) => renderRow(d, p)).join("")
      : `<tr><td colspan="9" class="empty">No funds yet. A fund is added at NAV and held.</td></tr>`,
    funds.length
      ? subtotal("Funds", sum(funds).invested, sum(funds).value)
      : "",
  );

  const content = `${renderHead(
    d.mode === "trade" ? "Short-term book" : "Holdings",
    d.mode === "trade"
      ? "Short-term pot · separate capital, separate limits"
      : `Long-term book · ${stocks.length} ${stocks.length === 1 ? "stock" : "stocks"} and ${funds.length} ${funds.length === 1 ? "fund" : "funds"}`,
    [
      { label: "Invested", value: money(all.invested, 0) },
      { label: "Now", value: money(all.value, 0) },
      { label: "Unrealised", value: `${pnl >= 0 ? "+" : "−"}${money(Math.abs(pnl), 0)}`, tone: pnl >= 0 ? "pos" : "neg" },
      { label: "Cash", value: money(d.available, 0) },
    ],
  )}
  <div class="mode-row">${renderModeTabs(d.mode)}<span class="sub">${d.currency} · as of the last poll</span></div>
  ${d.mode === "trade" ? section("Short-term positions", `${d.positions.length} · all stocks`, STOCK_HEAD,
      d.positions.length ? d.positions.map((p) => renderRow(d, p)).join("") : `<tr><td colspan="9" class="empty">Nothing in the short-term pot.</td></tr>`) : stocksSection + fundsSection}
  <div class="cards">
    <section class="card">
      <div class="lbl">Add an instrument</div>
      <div class="search"><input id="search" type="search" placeholder="Infosys, INFY, 120503, Motilal Oswal Midcap…"
        autocomplete="off" aria-label="Search stocks and mutual funds" />
      <div id="results" class="results" role="listbox" aria-label="Search results"></div></div>
      <div class="lbl" style="margin-top:var(--space-4)">Recent fills</div>
      <div class="feed">${history}</div>
    </section>
    <section class="card" id="decisions-card">
      <div class="card-head">
        <div class="lbl">Decisions</div>
        ${decisionFilterTabs(filter, `/?mode=${d.mode}`)}
      </div>
      <div class="feed decisions-feed">${advice}</div>
    </section>
  </div>`;

  return renderApp("ledger", content, {
    label: "Free to deploy",
    value: money(d.available, 0),
    note: d.budget > 0 ? `of ${money(d.budget, 0)}` : undefined,
  }, { ledger: String(d.positions.length) });
}
