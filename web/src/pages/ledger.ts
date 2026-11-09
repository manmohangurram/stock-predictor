import type { Dashboard, Mode } from "../api";
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
  // A fund has no price exit: it is sold at NAV once the return you asked for is reached, so its
  // exit plan is a NAV, derived from the profit expectation. Without one, say so rather than
  // leaving a blank that looks like a rendering bug.
  const fundExit = p.target_profit_pct
    ? `<span class="num">${money(p.avg_cost * (1 + p.target_profit_pct / 100))} → +${p.target_profit_pct}%</span>`
    : `<span class="muted">no target set</span>`;
  const plan = isFund
    ? fundExit
    : p.stop === null && p.target === null
      ? `<span class="muted">no exit set</span>`
      : `<span class="num">${p.stop === null ? "—" : money(p.stop)} → ${p.target === null ? "—" : money(p.target)}</span>`;
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
  <th scope="col">P&amp;L</th><th scope="col">Signal</th><th scope="col" class="hide-sm">Exit plan</th></tr>`;

const subtotal = (label: string, invested: number, value: number): string => {
  const pnl = value - invested;
  return `<tr class="subtotal"><td>${label}</td><td class="hide-sm">—</td><td class="hide-sm">—</td>
    <td class="num hide-sm">${money(invested)}</td><td>—</td>
    <td class="num hide-sm">${money(value)}</td>
    <td class="num ${pnl >= 0 ? "up" : "down"}">${pnl >= 0 ? "+" : "−"}${money(Math.abs(pnl))}</td>
    <td>—</td><td class="sub hide-sm">—</td></tr>`;
};

/** One decision: what it says, and the two things you can do about it. */
/**
 * A plan is one decision with several legs: read as a whole, approved as a whole.
 *
 * Approving opens one editable fill per leg — you rarely get the exact price a proposal named, and
 * a plan is not approved one stock at a time: half a swap is not a swap.
 */
export function renderPlan(legs: Dashboard["signals"]): string {
  const first = legs[0];
  const sells = legs.filter((l) => l.action === "SELL");
  const buys = legs.filter((l) => l.action === "BUY");
  const proceeds = sells.reduce((n, l) => n + (l.qty ?? 0) * (l.entry ?? 0), 0);
  const cost = buys.reduce((n, l) => n + (l.qty ?? 0) * (l.entry ?? 0), 0);
  const left = proceeds - cost;
  const leg = (l: Dashboard["signals"][number]) => `<div class="plan-leg">
    <span class="stamp" data-stamp="${l.action === "SELL" ? "sell" : "buy"}">${l.action}</span>
    <b>${escapeHtml(l.name || l.symbol)}</b>
    <span class="sub">${escapeHtml(l.symbol)}</span>
    <span class="num">${l.entry === null ? "" : money(l.entry)}</span>
    <span class="lbl">${l.qty === null || l.qty === 0 ? "size on approval" : `${units(l.qty)} units`}</span>
  </div>${l.rationale ? `<p class="plan-why">${escapeHtml(l.rationale)}</p>` : ""}`;
  const arithmetic =
    sells.length && buys.length
      ? `<p class="decision-note">${money(proceeds)} from the sale${sells.length > 1 ? "s" : ""} covers ${money(cost)}, and ${money(Math.abs(left))} is ${left >= 0 ? "left in" : "short of"} the purse.</p>`
      : "";
  const status =
    first.status === "pending"
      ? ""
      : `<span class="pill ${first.status === "approved" ? "ok" : "todo"}">${first.status}</span>`;
  const actions = first.status === "pending" ? planActions(first, legs) : "";
  return `<div class="feed-row decision-feed plan-card">
    <div class="decision-main">
      <div><span class="stamp" data-stamp="estimate">${
        first.plan_scope === "fund" ? "FUND PLAN" : "STOCK PLAN"
      }</span>
        <b>${legs.length} moves</b>
        ${status}
        <span class="sub">${histDate(first.created_at)}</span></div>
      ${legs.map(leg).join("")}
      ${arithmetic}
      ${first.plan_rationale ? `<p class="decision-prose">${escapeHtml(first.plan_rationale)}</p>` : ""}
      ${actions}
    </div>
  </div>`;
}

/** Approve and reject the plan, with a fill row per leg. */
function planActions(
  first: Dashboard["signals"][number],
  legs: Dashboard["signals"],
): string {
  // One header, then one aligned row per leg: a stamp column, the holding, then the two numbers.
  const fills = `<div class="plan-fill plan-fill-head" aria-hidden="true">
      <span></span><span>Holding</span><span>Quantity</span><span>Price</span>
    </div>${legs
      .map(
        (l) => `<div class="plan-fill">
        <span class="stamp" data-stamp="${l.action === "SELL" ? "sell" : "buy"}">${l.action}</span>
        <span class="plan-fill-name">${escapeHtml(l.name || l.symbol)}
          <span class="sub">${escapeHtml(l.symbol)}</span></span>
        <input name="qty-${l.id}" type="number" min="0" step="any" value="${l.qty ?? ""}"
               aria-label="Quantity for ${escapeHtml(l.name || l.symbol)}" />
        <input name="price-${l.id}" type="number" min="0" step="any" value="${l.entry ?? ""}"
               aria-label="Price for ${escapeHtml(l.name || l.symbol)}" />
      </div>`,
      )
      .join("")}`;
  return `<div class="decision-actions">
    <button class="btn icon-btn" type="button" data-approve="${first.id}" aria-expanded="false"
            title="Approve the plan — record the fill you actually got on each leg"
            aria-label="Approve the plan">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3.4 8.4l3 3 7-7" /></svg>
    </button>
    <button class="btn ghost icon-btn" type="button" data-reject="${first.id}" aria-expanded="false"
            title="Reject the plan — no money moves" aria-label="Reject the plan">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M4 4l8 8M12 4l-8 8" /></svg>
    </button>
  </div>
  <form class="approve-form" data-approve-form="${first.id}" data-plan="${legs.map((l) => l.id).join(",")}" hidden>
    <div class="plan-fills">${fills}</div>
    <div class="row">
      <button class="btn primary" type="submit">Post all fills</button>
      <button class="btn ghost" type="button" data-cancel="${first.id}">Cancel</button>
      <span class="help" data-status="${first.id}" role="status"></span>
    </div>
  </form>
  <form class="reject-form" data-reject-form="${first.id}" hidden>
    <div class="row">
      <input name="reason" placeholder="Why not? (optional)" aria-label="Reason" />
      <button class="btn" type="submit">Reject</button>
      <button class="btn ghost" type="button" data-cancel="${first.id}">Cancel</button>
      <span class="help" data-status="${first.id}" role="status"></span>
    </div>
  </form>`;
}

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
        ${
          s.qty === null || s.qty === 0
            ? `<span class="lbl">size on approval</span>`
            : `<span class="lbl">${units(s.qty)} units</span>`
        }
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

/**
 * Adding an instrument is a search you pick from, not a box you hope works: one dialog, grouped
 * results, and the keyboard does the rest.
 */
function addDialog(): string {
  return `<div class="add-dialog-body" hidden>
    <div class="dlg-search">
      <input id="search" type="search" autocomplete="off" aria-label="Search stocks and mutual funds"
             placeholder="Infosys, INFY, 120503, Motilal Oswal Midcap…" role="combobox"
             aria-expanded="true" aria-controls="results" />
      <span class="kbd">esc</span>
    </div>
    <div id="results" class="results dlg-results" role="listbox" aria-label="Search results"></div>
    <div class="dlg-foot"><span>↑↓ to move</span><span>⏎ to add</span><span>Esc to close</span></div>
  </div>`;
}

export function renderLedger(d: Dashboard): string {
  const stocks = d.positions.filter((p) => p.class === "stock");
  const funds = d.positions.filter((p) => p.class !== "stock");
  const all = sum(d.positions);
  const pnl = all.value - all.invested;

  // Rendered on the server of truth (the browser) and in tests: no window, no filter.
  const raw =
    typeof location !== "undefined"
      ? new URLSearchParams(location.search).get("decisions")
      : null;
  const filter: DecisionFilter = isDecisionFilter(raw) ? raw : "pending";
  // Blocked proposals sit with the decisions: they are what the app wanted to do and could not.
  const shown = [...d.signals, ...d.blocked].sort((a, b) => b.id - a.id);
  const plans = new Map<number, Dashboard["signals"]>();
  for (const s of shown) {
    if (s.plan_id) plans.set(s.plan_id, [...(plans.get(s.plan_id) ?? []), s]);
  }
  // A plan is the stock book's business; the fund scan is its own arithmetic. Two channels,
  // shown as two groups, so a fund decision never reads as part of a stock plan.
  const drawn: { html: string; fund: boolean }[] = [];
  const seen = new Set<number>();
  for (const s of shown.slice(0, 10)) {
    const plan = s.plan_id;
    if (plan && plans.has(plan)) {
      if (seen.has(plan)) continue;
      seen.add(plan);
      // Sells first, as they must be executed.
      const legs = plans
        .get(plan)!
        .slice()
        .sort((a, b) => (a.action === "SELL" ? -1 : 1) - (b.action === "SELL" ? -1 : 1));
      drawn.push({ html: renderPlan(legs), fund: false });
      continue;
    }
    drawn.push({ html: decisionRow(s), fund: s.class === "mutual_fund" });
  }
  const group = (fund: boolean) => drawn.filter((d) => d.fund === fund).map((d) => d.html).join("");
  const both = drawn.some((d) => d.fund) && drawn.some((d) => !d.fund);
  const advice = shown.length
    ? [
        drawn.some((d) => !d.fund)
          ? `${both ? `<div class="decision-group"><span class="lbl">Stocks</span></div>` : ""}${group(false)}`
          : "",
        drawn.some((d) => d.fund)
          ? `<div class="decision-group"><span class="lbl">Fund scan · arithmetic, no model call</span></div>${group(true)}`
          : "",
      ].join("")
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
    `${funds.length} · bought at NAV · exit at the NAV target`,
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
      {
        label: "Free to deploy",
        value: money(d.available, 0),
        note: d.budget > 0 ? `of ${money(d.budget, 0)}` : undefined,
      },
    ],
    `<button class="btn primary head-action" type="button" data-add-open
             aria-haspopup="dialog">+ Add instrument</button>`,
  )}
  <div class="mode-row">${renderModeTabs(d.mode)}<span class="sub">${d.currency} · as of the last poll</span></div>
  <section class="card" id="decisions-card">
    <div class="card-head">
      <div class="lbl">Decisions</div>
      ${decisionFilterTabs(filter, `/?mode=${d.mode}`)}
    </div>
    <div class="feed decisions-feed">${advice}</div>
  </section>
  ${d.mode === "trade" ? section("Short-term positions", `${d.positions.length} · all stocks`, STOCK_HEAD,
      d.positions.length ? d.positions.map((p) => renderRow(d, p)).join("") : `<tr><td colspan="8" class="empty">Nothing in the short-term pot.</td></tr>`) : stocksSection + fundsSection}
`;

  return renderApp(
    "ledger",
    `${content}${addDialog()}<dialog id="add-dialog" class="add-dialog"></dialog>`,
    { ledger: String(d.positions.length) },
  );
}
