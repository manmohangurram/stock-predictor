import type { Dashboard, Mode } from "../api";
import { renderApp, renderHead } from "../components/shell";
import { renderDecisionActions } from "../components/decision-actions";
import { decisionFilterTabs, isDecisionFilter, type DecisionFilter } from "../components/decision-filter";
import { day, escapeHtml, histDate, money, units } from "../format";
import { renderConfidence } from "../components/confidence";
import { actionStamp, stamp, statusPill, toneOf } from "../components/stamp";

/** The portfolio strip: the four numbers that decide whether anything is affordable. */
export function renderModeTabs(mode: Mode): string {
  const tab = (m: Mode, label: string) =>
    `<a class="mode-tab" href="/?mode=${m}" ${m === mode ? 'aria-current="page"' : ""}>${label}</a>`;
  return `<nav class="modes" aria-label="Strategy">
    ${tab("long", "Long-term")}${tab("trade", "Short-term")}
  </nav>`;
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
  const sign = (n: number) => (n === 0 ? "" : n > 0 ? "+" : "−");
  const pnlCell = live
    ? `${sign(pnl)}${money(Math.abs(pnl))}<div class="sub">${sign(pnlPct)}${Math.abs(pnlPct).toFixed(1)}%</div>`
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
    ? `<span class="signal-cell">${stamp(toneOf(s.action), s.action)}${renderConfidence(s.confidence)}</span>`
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
  <td class="right hide-sm num-text">${plan}</td>
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

const STOCK_HEAD = `<tr><th scope="col">Holding</th><th scope="col" class="hide-sm num">Qty</th>
  <th scope="col" class="hide-sm num">Bought</th><th scope="col" class="hide-sm num">Invested</th>
  <th scope="col" class="num">Now</th><th scope="col" class="hide-sm num">Value</th>
  <th scope="col" class="num">P&amp;L</th><th scope="col">Signal</th>
  <th scope="col" class="hide-sm num">Exit plan</th></tr>`;
const FUND_HEAD = `<tr><th scope="col">Scheme</th><th scope="col" class="hide-sm num">Units</th>
  <th scope="col" class="hide-sm num">Avg NAV</th><th scope="col" class="hide-sm num">Invested</th>
  <th scope="col" class="num">NAV today</th><th scope="col" class="hide-sm num">Value</th>
  <th scope="col" class="num">P&amp;L</th><th scope="col">Signal</th>
  <th scope="col" class="hide-sm num">Exit plan</th></tr>`;

const subtotal = (label: string, invested: number, value: number): string => {
  const pnl = value - invested;
  return `<tr class="subtotal"><td>${label}</td><td class="hide-sm">—</td><td class="hide-sm">—</td>
    <td class="num hide-sm">${money(invested)}</td><td>—</td>
    <td class="num hide-sm">${money(value)}</td>
    <td class="num ${pnl === 0 ? "" : pnl > 0 ? "up" : "down"}">${pnl === 0 ? "" : pnl > 0 ? "+" : "−"}${money(Math.abs(pnl))}</td>
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
  const isFund = first.plan_scope === "fund";
  const value = (l: Dashboard["signals"][number]) => l.amount ?? (l.qty ?? 0) * (l.entry ?? 0);
  const freed = legs.filter((l) => l.action === "SELL").reduce((n, l) => n + value(l), 0);
  const spent = legs.filter((l) => l.action === "BUY").reduce((n, l) => n + value(l), 0);

  const figure = (label: string, v: string) =>
    `<span class="docket-fig"><span class="lbl">${label}</span><b class="num">${v}</b></span>`;
  const leg = (l: Dashboard["signals"][number]) => {
    const buy = l.action === "BUY";
    const sized = l.qty !== null && l.qty > 0;
    const figs = sized
      ? [
          figure("Qty", units(l.qty!)),
          l.entry === null ? "" : figure("At", money(l.entry)),
          buy || l.entry === null ? "" : figure("Gives", money(value(l))),
          buy && l.stop !== null ? figure("Stop", money(l.stop)) : "",
          buy && l.target !== null ? figure("Target", money(l.target)) : "",
          buy && l.valid_until ? figure("Till", day(l.valid_until)) : "",
        ].join("")
      : l.amount !== null
        ? figure("Amount", money(l.amount))
        : "";
    return `<div class="docket-move">
      ${actionStamp(l.action)}
      <span class="docket-sym">${escapeHtml(l.name || l.symbol)}</span>
      <span class="docket-figs">${
        figs || `<span class="lbl">size on approval</span>`
      }</span>
    </div>`;
  };

  const row = (label: string, v: number, strong = false) =>
    `<div class="docket-row${strong ? " total" : ""}"><span class="lbl">${label}</span>
      <span class="num">${money(v)}</span></div>`;
  const spread = Math.abs(spent - freed) < 0.005 ? "" : spent > freed
    ? row("Also from the purse", spent - freed, true)
    : row("Back to the purse", freed - spent, true);
  const purse =
    freed > 0 && spent > 0
      ? `<div class="docket-purse">${row("From the sale", freed)}${row("To buy", spent)}${spread}</div>`
      : legs.length === 1 && first.amount !== null
        ? ""
        : spent > 0
          ? `<div class="docket-purse">${row("Cost", spent, true)}</div>`
          : freed > 0
            ? `<div class="docket-purse">${row("Frees", freed, true)}</div>`
            : "";

  const status = first.status === "pending" ? "" : statusPill(first.status);
  const actions = first.status === "pending" ? planActions(first, legs) : "";
  return `<div class="feed-row decision-feed plan-card">
    <div class="decision-main">
      <div>${stamp("estimate", isFund ? "FUND PLAN" : "STOCK PLAN")}
        <b>${legs.length} move${legs.length === 1 ? "" : "s"}</b>
        ${status}
        <span class="sub">${histDate(first.created_at)}</span></div>
      <div class="docket">${legs.map(leg).join("")}${purse}</div>
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
  const fundPlan = first.plan_scope === "fund";
  const fills = `<div class="plan-fill plan-fill-head" aria-hidden="true">
      <span></span><span>Holding</span><span>${fundPlan ? "Amount ₹" : "Quantity"}</span>
      <span>${fundPlan ? "NAV" : "Price"}</span>
    </div>${legs
      .map(
        (l) => `<div class="plan-fill">
        ${actionStamp(l.action)}
        <span class="plan-fill-name">${escapeHtml(l.name || l.symbol)}
          <span class="sub">${escapeHtml(l.symbol)}</span></span>
        <input name="${fundPlan ? "amount" : "qty"}-${l.id}" type="number" min="0" step="any"
               value="${fundPlan ? (l.amount ?? "") : (l.qty ?? "")}"
               aria-label="${fundPlan ? "Amount" : "Quantity"} for ${escapeHtml(l.name || l.symbol)}" />
        <input name="price-${l.id}" type="number" min="0" step="any" value="${l.entry ?? ""}"
               aria-label="${fundPlan ? "NAV" : "Price"} for ${escapeHtml(l.name || l.symbol)}" />
        ${fundPlan ? `<input type="hidden" name="qty-${l.id}" value="${l.qty ?? ""}" />` : ""}
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
      <button class="btn primary" type="submit">Post fills</button>
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
  const status =
    s.status === "pending"
      ? ""
      : statusPill(s.status);
  const actions = renderDecisionActions(s);
  // A fund's scheme code (0P0000XV5C.BO) says nothing; its name does. The code stays underneath,
  // small, because that is what you paste into the search box.
  const named = s.name && s.name !== s.symbol;
  return `<div class="feed-row decision-feed">
    <div class="decision-main">
      <div>${actionStamp(s.action)}
        <b title="${escapeHtml(s.symbol)}">${escapeHtml(named ? s.name : s.symbol)}</b>
        ${named ? `<span class="sub">${escapeHtml(s.symbol)}</span>` : ""}
        ${s.entry === null ? "" : `<span class="num">${money(s.entry)}</span>`}
        ${
          s.qty === null || s.qty === 0
            ? `<span class="lbl">size on approval</span>`
            : `<span class="lbl">${units(s.qty)} units</span>`
        }
        ${status}
        <span class="sub">${histDate(s.created_at)}</span></div>
      ${
        // The model's reasoning when there is one; the rules' reason when there is not — an exit at
        // the stop or at the idea's expiry has no rationale, and the card must still say why.
        s.rationale
          ? `<p class="decision-prose">${escapeHtml(s.rationale)}</p>`
          : s.reasons.length
            ? `<p class="decision-prose">${escapeHtml(s.reasons[0])}</p>`
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
  // A blocked proposal is not a decision to review: it is a record of something the app could not
  // do. It stays on the instrument's page and in the audit, not in this box.
  const shown = [...d.signals].sort((a, b) => b.id - a.id);
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
      {
        label: "Unrealised",
        value: pnl === 0 ? money(0, 0) : `${pnl > 0 ? "+" : "−"}${money(Math.abs(pnl), 0)}`,
        tone: pnl === 0 ? undefined : pnl > 0 ? "pos" : "neg",
      },
      {
        label: "Free to deploy",
        value: money(d.available, 0),
        note: d.budget > 0 ? `of ${money(d.budget, 0)}` : undefined,
      },
    ],
    `<button class="btn primary head-action" type="button" data-add-open
             aria-haspopup="dialog">Add instrument…</button>`,
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
