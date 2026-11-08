import type { Analysis, InstrumentDetail, Trade } from "../api";
import { clock, day, escapeHtml, histDate, money, units } from "../format";
import { confidenceLabel, renderConfidence } from "../components/confidence";
import { renderRisk } from "../components/risk";
import { stamp } from "../components/stamp";
import { renderApp } from "../components/shell";

function chart(a: Analysis): string {
  if (a.bars.length < 2) return `<p class="empty">No price history stored yet.</p>`;
  const w = 640;
  const h = 190;
  const pad = 8;
  const closes = a.bars.map((b) => b.close);
  const refs = a.indicators ? [a.indicators.sma50, a.indicators.sma200] : [];
  const lo = Math.min(...closes, ...refs);
  const hi = Math.max(...closes, ...refs);
  const span = hi - lo || 1;
  const x = (i: number) => pad + (i * (w - pad * 2)) / (closes.length - 1);
  const y = (v: number) => pad + ((hi - v) * (h - pad * 2)) / span;
  const path = closes
    .map((c, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(c).toFixed(1)}`)
    .join(" ");
  const ref = (v: number, label: string) => `
    <line x1="${pad}" x2="${w - pad}" y1="${y(v).toFixed(1)}" y2="${y(v).toFixed(1)}"
          stroke="var(--rule-strong)" stroke-dasharray="3 4" />
    <text x="${pad + 2}" y="${(y(v) - 4).toFixed(1)}" font-size="10"
          fill="var(--ink-tertiary)">${label}</text>`;

  return `<figure class="chart">
    <svg viewBox="0 0 ${w} ${h}" role="img" aria-label="${escapeHtml(a.instrument.symbol)} close, last ${closes.length} sessions">
      <line x1="${pad}" x2="${w - pad}" y1="${h - pad}" y2="${h - pad}" stroke="var(--rule)" />
      <path d="${path}" fill="none" stroke="var(--ink)" stroke-width="1.5" />
      ${a.indicators ? ref(a.indicators.sma50, "SMA50") : ""}
      ${a.indicators ? ref(a.indicators.sma200, "SMA200") : ""}
    </svg>
    <figcaption>${closes.length} sessions · last ${money(closes[closes.length - 1])}</figcaption>
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
  return `<form class="trade-form" data-symbol="${escapeHtml(symbol)}">
  <label class="lbl" for="side">Record a fill</label>
  <div class="row">
    <select id="side" name="side" aria-label="Buy or sell">
      <option value="buy">Buy</option>
      <option value="sell">Sell</option>
    </select>
    ${
      klass === "stock"
        ? `<select name="mode" aria-label="Which pot">
      <option value="long">Long-term pot</option>
      <option value="trade">Trading pot</option>
    </select>`
        : `<input type="hidden" name="mode" value="long" />
    <span class="lbl" title="A mutual fund is bought at NAV and held">Long-term pot only</span>`
    }
    <input name="qty" type="number" min="0" step="any"
           placeholder="Qty${held === null ? "" : ` (held ${units(held)})`}"
           data-held="${held ?? ""}" required aria-label="Quantity" />
    <input name="price" type="number" min="0" step="any" placeholder="Price" required aria-label="Price" />
    <input name="at" type="date" value="${today(now)}" max="${today(now)}"
           aria-label="Trade date" title="Change this to backfill an older fill" />
    <button class="btn" type="submit">Record</button>
  </div>
  <p class="help" id="trade-status" role="status">
    Defaults to today — change the date to backfill an older fill. A buy is limited by cash; a
    sell by what you hold${held === null ? "" : ` (${units(held)})`}.
  </p>
</form>`;
}

function decisions(d: InstrumentDetail, targetProfitPct: number | null): string {
  if (!d.signals.length) return `<p class="empty">No decisions recorded yet.</p>`;
  return d.signals
    .map(
      (s) => `<article class="decision">
  <header>
    <span class="stamp" data-stamp="${s.action === "SELL" ? "sell" : s.action === "BUY" ? "buy" : "blocked"}">${s.action}</span>
    <b class="num">${s.entry === null ? "—" : money(s.entry)}</b>
    ${s.qty === null ? "" : `<span class="lbl">${units(s.qty)} units</span>`}
    ${renderConfidence(s.confidence) ? `<span class="conf-word">${renderConfidence(s.confidence)}<span class="lbl">${confidenceLabel(s.confidence)}</span></span>` : ""}
    <time class="lbl">${day(s.created_at)} ${clock(s.created_at)}</time>
  </header>
  ${
    s.action === "BUY" && (s.stop !== null || s.target !== null)
      ? `<div class="exit-plan">
          ${s.stop === null ? "" : `<div><span class="lbl">Sell if it falls to</span><b class="num down">≤ ${money(s.stop)}</b></div>`}
          ${s.target === null ? "" : `<div><span class="lbl">Take profit at</span><b class="num up">≥ ${money(s.target)}</b></div>`}
        </div>`
      : ""
  }
  ${renderRisk(s, targetProfitPct)}
  ${s.rationale ? `<p class="why">${escapeHtml(s.rationale)}</p>` : ""}
  ${s.reasons.length ? `<ul>${s.reasons.map((r) => `<li>${escapeHtml(r)}</li>`).join("")}</ul>` : ""}
  <footer>
    stop ${s.stop === null ? "—" : money(s.stop)} · target ${s.target === null ? "—" : money(s.target)}
    ${s.valid_until ? ` · valid until ${day(s.valid_until)}` : ""}
    ${s.confidence === null ? " · rules-derived, no model score" : ""}
  </footer>
</article>`,
    )
    .join("");
}

function trades(list: Trade[]): string {
  if (!list.length) return `<p class="empty">No fills recorded for this instrument.</p>`;
  const vsClose = (t: Trade): string => {
    if (t.market_close === null) return `<span class="muted">—</span>`;
    const diff = (t.price - t.market_close) / t.market_close;
    const cls = diff > 0.0005 ? "up" : diff < -0.0005 ? "down" : "";
    return `<span class="num ${cls}">${diff >= 0 ? "+" : ""}${(diff * 100).toFixed(2)}%</span>`;
  };
  return `<table class="blotter compact">
    <thead><tr>
      <th scope="col">Date</th><th scope="col">Side</th><th scope="col">Qty</th>
      <th scope="col">Your price</th><th scope="col">Close that day</th>
      <th scope="col">Vs close</th><th scope="col">Realised</th>
    </tr></thead>
    <tbody>${list
      .map(
        (t) => `<tr>
      <td class="num">${histDate(t.traded_at)}</td>
      <td>${stamp(t.side === "buy" ? "buy" : "sell", t.side)}</td>
      <td class="num">${units(t.qty)}</td>
      <td class="num">${money(t.price)}</td>
      <td class="num">${t.market_close === null ? "—" : money(t.market_close)}</td>
      <td>${vsClose(t)}</td>
      <td class="num ${(t.realised ?? 0) < 0 ? "down" : (t.realised ?? 0) > 0 ? "up" : ""}">${t.realised === null ? "—" : `${t.realised >= 0 ? "+" : ""}${money(t.realised, 0)}`}</td>
    </tr>`,
      )
      .join("")}</tbody></table>`;
}

/** \`d\` may be the analysis payload itself: the API now returns the whole page in one call. */
export function renderInstrument(a: Analysis, d: InstrumentDetail, list: Trade[]): string {
  const p = a.position;
  const ltp = a.bars.length ? a.bars[a.bars.length - 1].close : (p?.avg_cost ?? null);
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

  return renderApp("ledger", `<div class="detail-stack">
  <header class="strip detail-head">
    <div class="who">
      <a class="back" href="/">← Ledger</a>
      <h1>${escapeHtml(a.instrument.symbol)}</h1>
      <span class="lbl">${escapeHtml(a.instrument.name)} · ${escapeHtml(a.instrument.class.replace("_", " "))}</span>
    </div>
    <div><span class="lbl">Last</span><b class="num total">${ltp === null ? "—" : money(ltp)}</b></div>
    <div><span class="lbl">Held</span><b class="num">${p ? `${units(p.qty)} @ ${money(p.avg_cost)}` : "—"}</b></div>
    <div><span class="lbl">Value</span><b class="num">${p && ltp !== null ? money(p.qty * ltp, 0) : "—"}</b></div>
  </header>
  <div class="body">
    <section class="main">
      ${chart(a)}
      ${renderTradeForm(
        a.instrument.symbol,
        p ? { qty: p.qty } : null,
        new Date(),
        a.instrument.class as "stock" | "mutual_fund" | "ipo",
      )}

      <h2 class="sect">Decisions</h2>
      ${decisions(d, a.instrument.target_profit_pct)}

      <h2 class="sect">Fills</h2>
      ${trades(list)}

      <h2 class="sect">Headlines</h2>
      ${headlines}
    </section>
    <aside class="side">
      <div class="lbl">Indicators</div>
      ${facts.map(([k, v]) => `<div class="pair"><span>${k}</span><span class="num">${v}</span></div>`).join("")}
      <div class="lbl" style="margin-top:16px">Profit expected from this holding</div>
      <form class="target-form" data-symbol="${escapeHtml(a.instrument.symbol)}">
        <input name="pct" type="number" min="0" step="0.5" placeholder="default"
               value="${a.instrument.target_profit_pct ?? ""}" aria-label="Expected profit percent" />
        <button class="btn" type="submit">Save</button>
      </form>
      <p class="cites">
        Blank uses the pot's minimum. Must stay inside the pot's band; the band's maximum always
        wins over a stock asking for more.
      </p>
      <div class="lbl" style="margin-top:16px">Rules</div>
      ${a.checks
        .map(
          (c) => `<div class="check"><span class="mark" data-passed="${c.passed}">${c.passed ? "PASS" : "BLOCK"}</span>
        <div><b>${escapeHtml(c.name)}</b><span class="detail">${escapeHtml(c.detail)}</span></div></div>`,
        )
        .join("")}
      <p class="cites" style="margin-top:14px">${escapeHtml(a.instrument.timing_note)}</p>
    </aside>
  </div></div>`);
}
