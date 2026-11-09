import type { Analysis, InstrumentDetail, Trade } from "../api";
import { clock, day, escapeHtml, money, units } from "../format";
import { confidenceLabel, renderConfidence } from "../components/confidence";
import { renderRisk } from "../components/risk";
import { renderApp } from "../components/shell";
import { renderDecisionActions } from "../components/decision-actions";

const klassOf = (c: string): "stock" | "mutual_fund" | "ipo" =>
  c === "mutual_fund" ? "mutual_fund" : c === "ipo" ? "ipo" : "stock";

/**
 * Daily close with real axes: prices down the left, dates along the bottom, and the two moving
 * averages drawn where they belong rather than listed in a panel.
 */
function chart(a: Analysis): string {
  if (a.bars.length < 2) {
    return `<p class="empty">No price history stored yet — open this instrument once the next poll has run.</p>`;
  }
  const w = 900;
  // 30% shorter than the first cut: the chart is a reading aid, not the page.
  const h = 196;
  const left = 68;
  const right = 18;
  const top = 12;
  const bottom = 26;
  const closes = a.bars.map((b) => b.close);
  const refs = a.indicators ? [a.indicators.sma50, a.indicators.sma200] : [];
  const rawLo = Math.min(...closes, ...refs);
  const rawHi = Math.max(...closes, ...refs);
  const air = (rawHi - rawLo || rawLo * 0.02) * 0.08;
  const lo = rawLo - air;
  const hi = rawHi + air;
  const span = hi - lo || 1;
  const x = (i: number) => left + (i * (w - left - right)) / (closes.length - 1);
  const y = (v: number) => top + ((hi - v) * (h - top - bottom)) / span;
  const path = closes
    .map((c, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(c).toFixed(1)}`)
    .join(" ");

  const ticks = [0, 1, 2, 3].map((n) => lo + ((hi - lo) * n) / 3);
  const grid = ticks
    .map(
      (v) => `<line x1="${left}" x2="${w - right}" y1="${y(v).toFixed(1)}" y2="${y(v).toFixed(1)}"
          stroke="var(--rule-subtle)" stroke-dasharray="2 4" />
      <text x="${left - 8}" y="${(y(v) + 3.5).toFixed(1)}" text-anchor="end" font-size="10"
        font-family="var(--font-ledger)" fill="var(--ink-tertiary)">${money(v, 0)}</text>`,
    )
    .join("");

  const marks = [0, 0.25, 0.5, 0.75, 1].map((f) => Math.round(f * (a.bars.length - 1)));
  const dates = marks
    .map((i, n) => {
      const anchor = n === 0 ? "start" : n === marks.length - 1 ? "end" : "middle";
      return `<text x="${x(i).toFixed(1)}" y="${h - 10}" text-anchor="${anchor}" font-size="10"
        font-family="var(--font-ledger)" fill="var(--ink-tertiary)">${day(a.bars[i].ts)}</text>`;
    })
    .join("");

  const ref = (v: number, label: string, tone: string) => `
    <line x1="${left}" x2="${w - right}" y1="${y(v).toFixed(1)}" y2="${y(v).toFixed(1)}"
          stroke="${tone}" stroke-dasharray="4 4" />
    <text x="${w - right - 2}" y="${(y(v) - 5).toFixed(1)}" text-anchor="end" font-size="10"
          font-family="var(--font-ledger)" fill="${tone}">${label} ${money(v, 0)}</text>`;

  return `<figure class="chart">
    <svg viewBox="0 0 ${w} ${h}" role="img"
         aria-label="${escapeHtml(a.instrument.name)}, close price, last ${closes.length} sessions">
      ${grid}
      <line x1="${left}" x2="${w - right}" y1="${h - bottom}" y2="${h - bottom}" stroke="var(--rule)" />
      <line x1="${left}" x2="${left}" y1="${top}" y2="${h - bottom}" stroke="var(--rule)" />
      <path d="${path} L${x(closes.length - 1).toFixed(1)},${h - bottom} L${left},${h - bottom} Z"
            fill="var(--stamp-ink)" opacity=".06" />
      <path d="${path}" fill="none" stroke="var(--stamp-ink)" stroke-width="1.75" />
      ${a.indicators ? ref(a.indicators.sma50, "SMA50", "var(--ink-tertiary)") : ""}
      ${a.indicators ? ref(a.indicators.sma200, "SMA200", "var(--stamp-oxide)") : ""}
      ${dates}
    </svg>
    <figcaption>${closes.length} sessions · daily close · last ${money(closes[closes.length - 1])}</figcaption>
  </figure>`;
}

/** Today in the user's own timezone, as the date input expects. */
export function today(now = new Date()): string {
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

/** Only a sale is capped by what is held. A buy is capped by cash, not by the position. */
export const qtyLimit = (side: "buy" | "sell", held: number | null): number | null =>
  side === "sell" && held !== null ? held : null;

export function renderTradeForm(
  symbol: string,
  position: { qty: number } | null,
  now = new Date(),
  /** A mutual fund is bought at NAV and held: the trading pot is not offered for it. */
  klass: "stock" | "mutual_fund" | "ipo" = "stock",
): string {
  const held = position?.qty ?? null;
  const option = (value: "buy" | "sell", label: string, checked: boolean) => `
    <label class="seg-option">
      <input type="radio" name="side" value="${value}" ${checked ? "checked" : ""} />
      <span>${label}</span>
    </label>`;
  return `<form class="trade-form" data-symbol="${escapeHtml(symbol)}">
  <div class="seg" role="radiogroup" aria-label="Buy or sell">
    ${option("buy", "Buy", true)}${option("sell", "Sell", false)}
  </div>
  <div class="fill-grid">
    <label class="field"><span class="lbl">Quantity</span>
      <input name="qty" type="number" min="0" step="any" required
             placeholder="${held === null ? "Qty" : `held ${units(held)}`}"
             data-held="${held ?? ""}" /></label>
    <label class="field"><span class="lbl">Price</span>
      <input name="price" type="number" min="0" step="any" required placeholder="Price" /></label>
    <label class="field"><span class="lbl">Date</span>
      <input name="at" type="date" value="${today(now)}" max="${today(now)}" /></label>
    <label class="field"><span class="lbl">Pot</span>
      ${
        klass === "stock"
          ? `<select name="mode"><option value="long">Long-term</option><option value="trade">Trading</option></select>`
          : `<input type="hidden" name="mode" value="long" /><span class="static">Long-term only</span>`
      }</label>
  </div>
  <button class="btn primary wide" type="submit">Record the fill</button>
  <p class="help" id="trade-status" role="status">
    A buy is limited by cash; a sell by what you hold${held === null ? "" : ` (${units(held)})`}.
  </p>
</form>`;
}

/** Decisions as a table whose rows open in place, carrying the whole message. */
function decisions(d: InstrumentDetail, targetProfitPct: number | null): string {
  if (!d.signals.length) return `<p class="empty">No decisions recorded yet.</p>`;
  const rows = d.signals
    .map((s, i) => {
      const detail = [
        s.qty === null ? "" : `<div><span class="lbl">Quantity</span><b class="num">${units(s.qty)}</b></div>`,
        s.stop === null ? "" : `<div><span class="lbl">Sell if it falls to</span><b class="num down">≤ ${money(s.stop)}</b></div>`,
        s.target === null ? "" : `<div><span class="lbl">Take profit at</span><b class="num up">≥ ${money(s.target)}</b></div>`,
        s.valid_until ? `<div><span class="lbl">Valid until</span><b class="num">${day(s.valid_until)}</b></div>` : "",
      ].join("");
      const tone = s.action === "SELL" ? "sell" : s.action === "BUY" ? "buy" : "blocked";
      return `<tr class="decision-row" data-decision="${i}" data-signal="${s.id}" tabindex="0" role="button"
            aria-expanded="false" aria-controls="decision-${i}">
        <td class="mono">${day(s.created_at)} ${clock(s.created_at)}</td>
        <td><span class="stamp" data-stamp="${tone}">${s.action}</span></td>
        <td class="num">${s.entry === null ? "—" : money(s.entry)}</td>
        <td class="num hide-sm">${s.stop === null ? "—" : money(s.stop)}</td>
        <td class="num hide-sm">${s.target === null ? "—" : money(s.target)}</td>
        <td class="hide-sm">${
          s.status !== "pending"
            ? `<span class="pill ${s.status === "approved" ? "ok" : "todo"}">${s.status}</span>`
            : s.confidence === null
            ? `<span class="muted">rules-derived, no model score</span>`
            : `<span class="conf-word">${renderConfidence(s.confidence)}<span class="lbl">${confidenceLabel(s.confidence)}</span></span>`
        }</td>
        <td class="right decision-row-actions">
          <button class="btn ghost" type="button" data-open="${i}" aria-expanded="false">Open</button>
          ${
            s.status === "pending" && ["BUY", "SELL"].includes(s.action)
              ? `<button class="btn" type="button" data-approve-row="${i}">Approve</button>`
              : ""
          }
        </td>
      </tr>
      <tr class="decision-detail" id="decision-${i}" hidden><td colspan="7">
        <div class="decision-body">
          ${detail ? `<div class="decision-grid">${detail}</div>` : ""}
          ${renderRisk(s, targetProfitPct)}
          ${
            s.rationale
              ? `<p class="decision-prose">${escapeHtml(s.rationale)}</p>`
              : `<p class="empty">No model rationale — this came from the rules alone.</p>`
          }
          ${s.reasons.length ? `<ul class="decision-reasons">${s.reasons.map((r) => `<li>${escapeHtml(r)}</li>`).join("")}</ul>` : ""}
          ${renderDecisionActions(s)}
        </div>
      </td></tr>`;
    })
    .join("");
  return `<div class="table-wrap"><table class="blotter decisions">
    <thead><tr><th scope="col">When</th><th scope="col">Action</th><th scope="col">Entry</th>
      <th scope="col" class="hide-sm">Stop</th><th scope="col" class="hide-sm">Target</th>
      <th scope="col" class="hide-sm">Confidence</th><th scope="col"></th></tr></thead>
    <tbody>${rows}</tbody></table></div>`;
}

function trades(list: Trade[]): string {
  if (!list.length) return `<p class="empty">No fills recorded for this instrument.</p>`;
  const vsClose = (t: Trade): string => {
    if (t.market_close === null) return `<span class="muted">—</span>`;
    const diff = (t.price - t.market_close) / t.market_close;
    const cls = diff > 0.0005 ? "up" : diff < -0.0005 ? "down" : "";
    return `<span class="num ${cls}">${diff >= 0 ? "+" : ""}${(diff * 100).toFixed(2)}%</span>`;
  };
  return `<div class="table-wrap"><table class="blotter">
    <thead><tr><th scope="col">Date</th><th scope="col">Side</th><th scope="col">Qty</th>
      <th scope="col">Your price</th><th scope="col" class="hide-sm">Close that day</th>
      <th scope="col" class="hide-sm">vs close</th></tr></thead>
    <tbody>${list
      .map(
        (t) => `<tr><td class="mono">${day(t.traded_at)}</td>
      <td><span class="stamp" data-stamp="${t.side}">${t.side}</span></td>
      <td class="num">${units(t.qty)}</td>
      <td class="num">${money(t.price)}</td>
      <td class="num hide-sm">${t.market_close === null ? "—" : money(t.market_close)}</td>
      <td class="hide-sm">${vsClose(t)}</td></tr>`,
      )
      .join("")}</tbody></table></div>`;
}

/** The two levels that matter, with the money on each side. */
function exitPlan(p: NonNullable<Analysis["position"]>, ltp: number | null): string {
  const basis = ltp ?? p.avg_cost;
  const risk = p.stop === null ? null : (p.stop - basis) * p.qty;
  const reward = p.target === null ? null : (p.target - basis) * p.qty;
  const ratio = risk !== null && reward !== null && risk !== 0 ? Math.abs(reward / risk) : null;
  const row = (label: string, value: string, tone = "") =>
    `<div class="pair"><span>${label}</span><b class="num ${tone}">${value}</b></div>`;
  return `<div class="plan">
    ${p.stop === null ? "" : row("Sell if it falls to", money(p.stop), "down")}
    ${p.target === null ? "" : row("Take profit at", money(p.target), "up")}
    ${risk === null ? "" : row("Money at risk", `${risk >= 0 ? "+" : "−"}${money(Math.abs(risk))}`, "down")}
    ${reward === null ? "" : row("Reward if it works", `${reward >= 0 ? "+" : "−"}${money(Math.abs(reward))}`, "up")}
    ${ratio === null ? "" : row("Reward : risk", `${ratio.toFixed(1)} : 1`)}
  </div>`;
}

/** The instrument page: the chart and its evidence, with the numbers you act on beside it. */
export function renderInstrument(a: Analysis, d: InstrumentDetail, list: Trade[]): string {
  const p = a.position;
  const ltp = a.bars.length ? a.bars[a.bars.length - 1].close : (p?.avg_cost ?? null);
  const klass = klassOf(a.instrument.class);
  const facts = a.indicators
    ? [
        ["SMA50", money(a.indicators.sma50)],
        ["SMA200", money(a.indicators.sma200)],
        ["RSI14", a.indicators.rsi14.toFixed(1)],
        ["ATR14", money(a.indicators.atr14)],
        ["52w low", money(a.indicators.low52)],
        ["52w high", money(a.indicators.high52)],
      ]
    : [];
  const headlines = d.headlines.length
    ? `<ul class="headlines">${d.headlines
        .map(
          (h) => `<li><a href="${escapeHtml(h.url)}" target="_blank" rel="noreferrer noopener">${escapeHtml(h.title)}</a>
      <span class="cites">${escapeHtml(h.publisher ?? "")} · ${day(h.published_at)}</span></li>`,
        )
        .join("")}</ul>`
    : `<p class="empty">No headlines stored yet.</p>`;

  const invested = p ? p.qty * p.avg_cost : null;
  const value = p && ltp !== null ? p.qty * ltp : null;
  const unrealised = invested !== null && value !== null ? value - invested : null;

  const rules = a.checks.length
    ? `<div class="rules">${a.checks
        .map(
          (c) => `<div class="rule-row"><span class="pill ${c.passed ? "ok" : "todo"}">${c.passed ? "pass" : "block"}</span>
        <span>${escapeHtml(c.name)} <span class="sub">${escapeHtml(c.detail)}</span></span></div>`,
        )
        .join("")}</div>`
    : `<p class="empty">Not enough data for a rule check yet.</p>`;

  return renderApp(
    "ledger",
    `<header class="page-head">
      <div class="who">
        <a class="back" href="/">← Ledger</a>
        <h1>${escapeHtml(a.instrument.name)}</h1>
        <div class="page-sub">${escapeHtml(a.instrument.symbol)} · ${escapeHtml(a.instrument.class.replace("_", " "))}</div>
      </div>
      <div class="figures">
        <div><span class="lbl">Last</span><b>${ltp === null ? "—" : money(ltp)}</b></div>
        <div><span class="lbl">Held</span><b>${p ? `${units(p.qty)} @ ${money(p.avg_cost)}` : "—"}</b></div>
        <div><span class="lbl">Value</span><b>${value === null ? "—" : money(value)}</b></div>
        <div><span class="lbl">Unrealised</span><b class="${
          unrealised === null ? "" : unrealised >= 0 ? "up" : "down"
        }">${unrealised === null ? "—" : `${unrealised >= 0 ? "+" : "−"}${money(Math.abs(unrealised))}`}</b></div>
      </div>
    </header>
    <div class="inst-grid">
      <div class="inst-main">
        <section class="card chart-card">
          <div class="card-head"><h2>Price</h2><span class="sub">${a.bars.length} sessions · daily close</span></div>
          ${chart(a)}
        </section>
        <section class="card">
          <div class="card-head"><h2>Decisions</h2><span class="sub">open a row for the whole message</span></div>
          ${decisions(d, a.instrument.target_profit_pct)}
        </section>
        ${
          list.length
            ? `<section class="card">
          <div class="card-head"><h2>Fills</h2><span class="sub">what you actually paid</span></div>
          ${trades(list)}
        </section>`
            : `<section class="card compact">
          <div class="card-head"><h2>Fills</h2>
            <span class="sub">no fills yet — record the first one on the right</span></div>
        </section>`
        }
        <section class="card">
          <div class="card-head"><h2>Headlines</h2></div>
          ${headlines}
        </section>
      </div>
      <aside class="inst-side">
        <section class="card">
          <div class="card-head"><h2>Record a fill</h2><span class="sub">held ${p ? units(p.qty) : "0"}</span></div>
          ${renderTradeForm(a.instrument.symbol, p ? { qty: p.qty } : null, new Date(), klass)}
        </section>
        <section class="card">
          <div class="card-head"><h2>Exit plan</h2></div>
          ${
            p && (p.stop !== null || p.target !== null)
              ? exitPlan(p, ltp)
              : `<p class="empty">${
                  klass === "mutual_fund"
                    ? "A fund has no intraday exit: units are allotted at NAV."
                    : "No stop or target recorded yet."
                }</p>`
          }
        </section>
        <section class="card">
          <div class="card-head"><h2>Profit expected</h2></div>
          <form class="target-form" data-symbol="${escapeHtml(a.instrument.symbol)}">
            <input name="pct" type="number" min="0" step="0.5" placeholder="default"
                   value="${a.instrument.target_profit_pct ?? ""}" aria-label="Expected profit percent" />
            <button class="btn" type="submit">Save</button>
          </form>
          <p class="help">Blank uses the pot's minimum; the band's maximum always wins.</p>
        </section>
        <section class="card">
          <div class="card-head"><h2>What the rules see</h2></div>
          ${rules}
        </section>
        <section class="card">
          <div class="card-head"><h2>Manage</h2></div>
          <button class="btn ghost" type="button" id="delete-instrument"
                  data-symbol="${escapeHtml(a.instrument.symbol)}" data-held="${p ? p.qty : 0}">
            ${p ? "Sell it before deleting" : "Delete this instrument"}
          </button>
          <p class="help" id="delete-status" role="status">
            ${
              p
                ? "An instrument you hold cannot be removed — the history is what makes the ledger true."
                : "If it has fills, it is archived rather than deleted: hidden from polling, history kept."
            }
          </p>
        </section>
        <section class="card">
          <div class="card-head"><h2>Indicators</h2></div>
          <div class="facts">${facts.map(([k, v]) => `<div class="pair"><span>${k}</span><span class="num">${v}</span></div>`).join("")}</div>
        </section>
      </aside>
    </div>`,
  );
}
