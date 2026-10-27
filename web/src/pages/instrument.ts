import type { Analysis } from "../api";
import { day, escapeHtml, money } from "../format";

/** One ink line, two dotted references. No candles, no volume, no green/red. */
function chart(a: Analysis): string {
  if (a.bars.length < 2) return `<p class="empty">No price history stored yet.</p>`;
  const w = 640;
  const h = 180;
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
  const refLine = (v: number, label: string) => `
    <line x1="${pad}" x2="${w - pad}" y1="${y(v).toFixed(1)}" y2="${y(v).toFixed(1)}"
          stroke="var(--rule-strong)" stroke-dasharray="3 4" />
    <text x="${w - pad}" y="${(y(v) - 4).toFixed(1)}" text-anchor="end"
          font-size="10" fill="var(--ink-tertiary)">${label} ${money(v, 0)}</text>`;

  return `<figure class="chart">
    <svg viewBox="0 0 ${w} ${h}" role="img" aria-label="${escapeHtml(a.instrument.symbol)} close, last ${closes.length} sessions">
      <line x1="${pad}" x2="${w - pad}" y1="${h - pad}" y2="${h - pad}" stroke="var(--rule)" />
      <path d="${path}" fill="none" stroke="var(--ink)" stroke-width="1.5" />
      ${a.indicators ? refLine(a.indicators.sma50, "SMA50") : ""}
      ${a.indicators ? refLine(a.indicators.sma200, "SMA200") : ""}
    </svg>
    <figcaption>${closes.length} sessions · last ${money(closes[closes.length - 1])}</figcaption>
  </figure>`;
}

export function renderInstrument(a: Analysis): string {
  const checks = a.checks
    .map(
      (c) => `<tr class="check-row">
  <td><span class="mark" data-passed="${c.passed}">${c.passed ? "PASS" : "BLOCKED"}</span></td>
  <td>${escapeHtml(c.name)}</td>
  <td class="detail">${escapeHtml(c.detail)}</td>
</tr>`,
    )
    .join("");

  const facts = a.indicators
    ? [
        ["SMA50", money(a.indicators.sma50)],
        ["SMA200", money(a.indicators.sma200)],
        ["RSI14", a.indicators.rsi14.toFixed(1)],
        ["ATR14", money(a.indicators.atr14)],
        ["52w low", money(a.indicators.low52)],
        ["52w high", money(a.indicators.high52)],
      ]
        .map(([k, v]) => `<div class="pair"><span>${k}</span><span class="num">${v}</span></div>`)
        .join("")
    : `<p class="empty">Not enough stored bars for the rules (200 daily closes).</p>`;

  const headlines = a.headlines.length
    ? `<ul class="headlines">${a.headlines
        .map(
          (h) => `<li><a href="${escapeHtml(h.url)}" rel="noreferrer noopener" target="_blank">${escapeHtml(
            h.title,
          )}</a><span class="cites">${escapeHtml(h.publisher ?? "")} · ${day(h.published_at)}</span></li>`,
        )
        .join("")}</ul>`
    : `<p class="empty">No headlines stored for this instrument yet.</p>`;

  const position = a.position
    ? `<div class="pair"><span>Held</span><span class="num">${money(a.position.qty, 3)} @ ${money(
        a.position.avg_cost,
      )}</span></div>`
    : `<p class="empty">Not held.</p>`;

  return `<main id="main" class="page">
  <header class="header">
    <h1>${escapeHtml(a.instrument.symbol)} <span class="class-chip">${escapeHtml(
      a.instrument.class.replace("_", " "),
    )}</span></h1>
    <div class="nav">
      <a href="/">Ledger</a><a href="/positions">Positions</a>
      <a href="/settings">Settings</a><a href="/calls">Audit</a>
    </div>
    <div class="double-rule"></div>
  </header>
  <div class="ledger-col">
    ${chart(a)}
    <table class="ledger">
      <caption>What the rules see — every row must pass for a signal</caption>
      <tbody>${checks}</tbody>
    </table>
    <section>
      <h2 class="label">Headlines</h2>
      ${headlines}
    </section>
  </div>
  <aside class="rail">
    <span class="label">${escapeHtml(a.instrument.name)}</span>
    ${facts}
    ${position}
    <p class="cites">${escapeHtml(a.instrument.timing_note)}</p>
  </aside>
</main>`;
}
