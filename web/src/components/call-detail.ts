/**
 * The stored exchange, rendered so a person can read it: the prompt as prose, the verdict as a
 * card, the raw bodies a click away. Kept as pure functions — the view decides where it goes.
 */
import type { CallRow, ScheduledPrompt } from "../api";
import { day, dayTime, escapeHtml, humanLabel, money, titleCase, units } from "../format";
import { renderMarkdown } from "../markdown";
import { actionStamp, stamp } from "./stamp";
import {
  declaredTools,
  parseReply,
  pretty,
  promptBlocks,
  replyText,
  searchQuery,
  searchResults,
  type CallDetail,
} from "./call-viewer";

/** The reply as a decision, not as JSON: a verdict, its numbers, and what it wants next. */
/** One answer card, always the same shape: a verdict head, its figures or moves, then its words. */
function decisionCard(reply: string, when: string, names: Record<string, string>): string {
  const r = parseReply(reply);
  const head = (label: string, tone: string, title: string) =>
    `<div class="answer-head">
      ${stamp(tone as "buy" | "sell" | "estimate", label)}
      <span class="answer-title">${title}</span>
      <span class="answer-when">${escapeHtml(when)}</span>
    </div>`;

  if (r.moves.length) {
    const figure = (label: string, value: string) =>
      `<span class="docket-fig"><span class="lbl">${label}</span><b class="num">${value}</b></span>`;
    const legs = r.moves
      .map((m) => {
        const buy = m.action === "BUY";
        const value = m.amount ?? (m.qty !== null && m.price !== null ? m.qty * m.price : null);
        const figs = [
          m.amount !== null ? figure("Amount", money(m.amount)) : "",
          m.qty === null ? "" : figure("Qty", units(m.qty)),
          m.price === null ? "" : figure("At", money(m.price)),
          buy ? "" : value === null ? "" : figure("Gives", money(value)),
          buy && m.stop !== null ? figure("Stop", money(m.stop)) : "",
          buy && m.target !== null ? figure("Target", money(m.target)) : "",
          buy && m.validUntil ? figure("Till", day(m.validUntil)) : "",
        ].join("");
        return `<div class="docket-move">
          ${actionStamp(m.action)}
          <span class="docket-sym">${escapeHtml(names[m.symbol] || m.symbol)}</span>
          <span class="docket-figs">${figs}</span>
        </div>`;
      })
      .join("");

    const total = (sell: boolean) =>
      r.moves.reduce(
        (n, m) =>
          n +
          ((m.action === "BUY") !== sell
            ? (m.amount ?? (m.qty !== null && m.price !== null ? m.qty * m.price : 0))
            : 0),
        0,
      );
    const freed = total(true);
    const spent = total(false);
    const row = (label: string, value: number, strong = false) =>
      `<div class="docket-row${strong ? " total" : ""}"><span class="lbl">${label}</span>
        <span class="num">${money(value)}</span></div>`;
    const purse =
      freed > 0 && spent > 0
        ? `<div class="docket-purse">${row("From the sale", freed)}${row("To buy", spent)}
            ${
              Math.abs(spent - freed) < 0.005
                ? ""
                : row(
                    spent > freed ? "Also from the purse" : "Back to the purse",
                    Math.abs(spent - freed),
                    true,
                  )
            }</div>`
        : r.moves.length === 1 && r.moves[0].amount !== null
          ? ""
          : spent > 0
            ? `<div class="docket-purse">${row("Cost", spent, true)}</div>`
            : freed > 0
              ? `<div class="docket-purse">${row("Frees", freed, true)}</div>`
              : "";

    return `<article class="answer" data-verdict="plan">
      ${head("PLAN", "estimate", `${r.moves.length} move${r.moves.length === 1 ? "" : "s"}`)}
      <div class="answer-body">
        <div class="docket">${legs}${purse}</div>
        ${r.rationale ? `<p class="answer-prose">${escapeHtml(r.rationale)}</p>` : ""}
      </div>
    </article>`;
  }

  // An empty move list is a real answer: the model looked and proposed nothing.
  if (Array.isArray(r.rest?.moves)) {
    return `<article class="answer" data-verdict="pass">
      ${head("PASS", "estimate", "No moves")}
      <div class="answer-body">${
        r.why ? `<p class="answer-prose">${escapeHtml(r.why)}</p>` : ""
      }</div>
    </article>`;
  }

  if (!r.verdict) {
    return `<article class="answer" data-verdict="raw">
      ${head("UNPARSED", "estimate", "The reply did not parse")}
      <div class="answer-body"><pre class="answer-raw">${
        reply ? escapeHtml(reply) : "<em>no readable content — see the raw JSON</em>"
      }</pre></div>
    </article>`;
  }

  const tone = r.verdict === "BUY" ? "buy" : r.verdict === "SELL" ? "sell" : "estimate";
  const figure = (label: string, value: string) =>
    `<div class="answer-figure"><span class="lbl">${label}</span><b class="num">${value}</b></div>`;
  const figures = [
    r.qty !== null ? figure("Quantity", String(r.qty)) : "",
    r.entry !== null ? figure("Entry", money(r.entry)) : "",
    r.stop !== null ? figure("Stop", money(r.stop)) : "",
    r.target !== null ? figure("Target", money(r.target)) : "",
    r.confidence ? figure("Confidence", r.confidence) : "",
  ].join("");
  const wants = r.wantsHeadlines.length
    ? `<p class="answer-note">Asked for headline ${r.wantsHeadlines.join(", ")}${
        r.wantsHistory ? " and the 7-day history" : ""
      } before deciding.</p>`
    : "";
  const cites = r.cites.length
    ? `<div class="answer-cites">${r.cites
        .map((c) => `<span class="chip">source ${c}</span>`)
        .join("")}</div>`
    : "";
  return `<article class="answer" data-verdict="${r.verdict.toLowerCase()}">
    ${head(r.verdict, tone, r.verdict === "PASS" ? "Passed" : "Decision")}
    <div class="answer-body">
      ${figures ? `<div class="answer-figures">${figures}</div>` : ""}
      ${r.rationale || r.why ? `<p class="answer-prose">${escapeHtml(r.rationale ?? r.why ?? "")}</p>` : ""}
      ${wants}
      ${cites}
    </div>
  </article>`;
}

function firstWord(text: string | null): string {
  return (text ?? "").split(/\s+/)[0] || "—";
}

/** A copy button, and the full text it copies kept beside it — the view may truncate, a copy must not. */
function copyBlock(title: string, raw: string | null): string {
  const full = raw ?? "";
  return `<section class="call-block">
    <div class="call-block-head">
      <h3 class="call-h">${title}</h3>
      <button type="button" class="copy" data-copy title="Copy all of it" aria-label="Copy ${title}">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" fill="none"
          stroke="currentColor" stroke-width="1.3" stroke-linejoin="round">
          <rect x="5.5" y="5.5" width="8" height="8" rx="1.2"></rect>
          <path d="M10.5 5.5V3.2A1.2 1.2 0 0 0 9.3 2H3.2A1.2 1.2 0 0 0 2 3.2v6.1a1.2 1.2 0 0 0 1.2 1.2h2.3"></path>
        </svg>
      </button>
    </div>
    <pre>${escapeHtml(pretty(full))}</pre>
    <div class="copy-src" hidden>${escapeHtml(full)}</div>
  </section>`;
}

/** A search call: the query that was asked, and the results that came back, ranked. */
function searchView(d: CallDetail): string {
  const { query, count } = searchQuery(d.request_json);
  const results = searchResults(d.response_json);
  const rows = results
    .map((r, i) => {
      const when = r.date ? dayTime(r.date) : "";
      return `<li class="search-hit">
        <div class="search-head">
          <span class="search-n">${i + 1}</span>
          <span class="search-title">${escapeHtml(r.title || "(no title)")}</span>
        </div>
        <div class="search-src">${escapeHtml([r.site, when].filter(Boolean).join(" · "))}</div>
        <p class="search-gist">${escapeHtml(r.snippet || r.summary)}</p>
      </li>`;
    })
    .join("");
  return `<div class="call">
  <div class="call-bar">
    ${d.purpose ? `<span class="call-fact"><span class="lbl">Searched</span><b>${escapeHtml(query || d.purpose)}</b></span>` : ""}
    ${count ? `<span class="call-fact"><span class="lbl">Asked for</span><b>${count} results</b></span>` : ""}
    <span class="call-fact"><span class="lbl">Got</span><b>${results.length} results</b></span>
    <span class="call-fact"><span class="lbl">Time</span><b>${escapeHtml(dayTime(d.ts))}</b></span>
    ${d.error ? `<span class="call-fact refused"><span class="lbl">Failed</span><b>${escapeHtml(d.error)}</b></span>` : ""}
  </div>
  <section class="call-sec">
    <h3 class="call-h">What the search returned</h3>
    ${
      rows
        ? `<ol class="search-list">${rows}</ol>`
        : `<p class="call-empty">${
            d.error ? "The search failed, so there are no results." : "No results for this query."
          }</p>`
    }
  </section>
  <details class="raw">
    <summary>Raw request and response</summary>
    <div class="call-exchange">
      ${copyBlock("request_json", d.request_json)}
      ${copyBlock("response_json", d.response_json)}
    </div>
  </details>
</div>`;
}

export function renderExchangeDetail(d: CallDetail): string {
  if (d.endpoint === "search" || d.endpoint === "fetch") return searchView(d);
  const blocks = promptBlocks(d.request_json);
  const reply = replyText(d.response_json);
  const tools = declaredTools(d.request_json);
  const prompt = blocks.length
    ? blocks
        .map(
          (b) => `<section class="call-block">
      <div class="lbl">${escapeHtml(b.label)}</div>
      <div class="call-md">${renderMarkdown(b.text)}</div>
    </section>`,
        )
        .join("")
    : `<p class="call-empty">The request body is not a message list — see the raw JSON.</p>`;

  const fact = (label: string, value: string) =>
    `<span class="call-fact"><span class="lbl">${label}</span><b>${value}</b></span>`;

  return `<div class="call">
  <div class="call-bar">
    ${fact("Model", escapeHtml(humanLabel(d.prompt_template ?? firstWord(d.purpose))))}
    ${fact("Time", escapeHtml(dayTime(d.ts)))}
    ${d.round ? fact("Round", String(d.round)) : ""}
    ${fact("Tokens", `${d.prompt_tokens ?? 0} in · ${d.completion_tokens ?? 0} out`)}
    ${tools.length ? fact("Tools", escapeHtml(tools.join(", "))) : ""}
    ${d.trigger ? `<span class="call-fact why"><span class="lbl">Asked because</span><b>${escapeHtml(d.trigger)}</b></span>` : ""}
    ${d.decision_reason ? `<span class="call-fact refused"><span class="lbl">Refused</span><b>${escapeHtml(d.decision_reason)}</b></span>` : ""}
    ${d.error ? `<span class="call-fact refused"><span class="lbl">Failed</span><b>${escapeHtml(d.error)}</b></span>` : ""}
  </div>

  <section class="call-sec call-exchange">
    <div class="call-col">
      <h3 class="call-h">Sent to the model</h3>
      ${prompt}
    </div>
    <div class="call-col">
      <h3 class="call-h">Received</h3>
      ${
        // The card is the answer. The JSON it was parsed from is one click away in Raw, below.
        d.error || !reply
          ? `<p class="call-empty">${d.error ? "The call failed before a reply." : "No reply stored for this call."}</p>`
          : decisionCard(reply, dayTime(d.ts), d.plan_names ?? {})
      }
    </div>
  </section>

  <details class="raw">
    <summary>Raw request and response</summary>
    <div class="call-exchange">
      ${copyBlock("request_json", d.request_json)}
      ${copyBlock("response_json", d.response_json)}
    </div>
  </details>
</div>`;
}

/** One answer row: the action, what it is against, and the numbers that make it mean something. */
function parsedTable(
  legs: {
    action: string;
    symbol: string;
    qty: number | null;
    price: number | null;
    stop: number | null;
    target: number | null;
    amount: number | null;
  }[],
  names: Record<string, string>,
  fallback: string | null,
): string {
  if (!legs.length) return "";
  const byAmount = legs.some((l) => l.amount !== null);
  const rows = legs
    .map(
      (l) => `<tr>
        <td>${l.action ? actionStamp(l.action) : "—"}</td>
        <td>${escapeHtml(names[l.symbol] || l.symbol || fallback || "—")}</td>
        <td class="r num">${l.amount !== null ? money(l.amount) : l.qty !== null ? units(l.qty) : "—"}</td>
        <td class="r num">${l.price !== null ? money(l.price) : "—"}</td>
        <td class="r num hide-compact">${l.stop !== null ? money(l.stop) : "—"}</td>
        <td class="r num hide-compact">${l.target !== null ? money(l.target) : "—"}</td>
      </tr>`,
    )
    .join("");
  return `<table class="mtable">
    <thead><tr>
      <th>Action</th><th>Instrument</th>
      <th class="r">${byAmount ? "Amount" : "Qty"}</th>
      <th class="r">At</th>
      <th class="r hide-compact">Stop</th>
      <th class="r hide-compact">Target</th>
    </tr></thead>
    <tbody>${rows}</tbody>
  </table>`;
}

/** A search's parsed view: what was asked, and what came back, in the order the service ranked it. */
function parsedSearch(d: CallDetail, compact: boolean): string {
  const { query } = searchQuery(d.request_json);
  const results = searchResults(d.response_json);
  const fields = `<div class="pfield"><span>Query</span><b>${escapeHtml(query || "—")}</b></div>
    <div class="pfield"><span>Results</span><b>${results.length}</b></div>`;
  const table = results.length
    ? `<table class="mtable">
        <thead><tr><th>Result</th><th>Site</th><th class="r">Date</th></tr></thead>
        <tbody>${results
          .map(
            (r) => `<tr>
              <td><a href="${escapeHtml(r.url)}" target="_blank" rel="noreferrer">${escapeHtml(r.title || r.url)}</a></td>
              <td>${escapeHtml(r.site || "—")}</td>
              <td class="r num">${escapeHtml(r.date ?? "—")}</td>
            </tr>`,
          )
          .join("")}</tbody>
      </table>`
    : `<pre class="creply">${escapeHtml(replyText(d.response_json) || "No results stored for this call.")}</pre>`;

  if (compact) {
    return `<div class="callparsed compact">
      <h4>${escapeHtml(d.prompt_template ?? firstWord(d.purpose))} · search</h4>
      <div class="c-label">Prompt</div>
      <div class="md clamp">${renderMarkdown(query || d.purpose || "")}</div>
      <div class="c-label c-gap">Result</div>
      ${table}
      ${fields}
    </div>`;
  }
  return `<div class="callparsed">
    <div class="cgrid">
      <div><div class="c-label">Prompt · rendered</div>
        <div class="pblock"><div class="md">${renderMarkdown(query || d.purpose || "")}</div></div>
      </div>
      <div><div class="c-label">Result · parsed</div>${table}${fields}</div>
    </div>
  </div>`;
}

/** The audit drawer's parsed view: the prompt as prose, the answer as a table and its fields. */
export function renderParsed(d: CallDetail, compact = false): string {
  if (d.endpoint === "search" || d.endpoint === "fetch") return parsedSearch(d, compact);

  const reply = replyText(d.response_json);
  const r = parseReply(reply);
  const blocks = promptBlocks(d.request_json);
  const names = d.plan_names ?? {};

  const legs = r.moves.length
    ? r.moves
    : r.verdict || r.qty !== null || r.entry !== null
      ? [
          {
            action: r.verdict ?? "",
            symbol: d.instrument_symbol ?? "",
            qty: r.qty,
            price: r.entry,
            stop: r.stop,
            target: r.target,
            amount: null,
          },
        ]
      : [];
  const table = parsedTable(legs, names, d.instrument_name);

  // A short-term pass answers with candidates, not a trade; that is still a parsed result.
  const raw = (r.rest as Record<string, unknown> | null)?.candidates;
  const cands = Array.isArray(raw)
    ? raw.flatMap((c) => {
        const rec = c as Record<string, unknown> | null;
        const symbol = typeof rec?.symbol === "string" ? rec.symbol : "";
        if (!symbol) return [];
        return [{ symbol, why: String(rec?.why ?? rec?.reason ?? "") }];
      })
    : [];
  const candTable = cands.length
    ? `<table class="mtable">
        <thead><tr><th>Candidate</th><th>Why</th></tr></thead>
        <tbody>${cands
          .map(
            (c) => `<tr><td>${escapeHtml(c.symbol)}</td><td>${escapeHtml(c.why || "—")}</td></tr>`,
          )
          .join("")}</tbody>
      </table>`
    : "";

  const fields: [string, string][] = [];
  if (cands.length) fields.push(["Candidates", String(cands.length)]);
  if (r.verdict) fields.push(["Verdict", r.verdict]);
  if (r.confidence) fields.push(["Confidence", r.confidence]);
  if (d.validator_verdict) fields.push(["Review", d.validator_verdict]);
  fields.push(["Template", d.prompt_template ?? "—"]);
  fields.push(["Tokens", `${d.prompt_tokens ?? 0} in · ${d.completion_tokens ?? 0} out`]);
  const fieldHtml = fields
    .map(
      ([k, v]) => `<div class="pfield"><span>${escapeHtml(k)}</span><b>${escapeHtml(v)}</b></div>`,
    )
    .join("");
  const why = r.rationale ? `<p class="cwhy">${escapeHtml(r.rationale)}</p>` : "";

  const parsed = legs.length > 0 || r.verdict !== null || cands.length > 0;
  const unparsed = parsed
    ? ""
    : `<pre class="creply">${escapeHtml(reply || "No reply stored for this call.")}</pre>`;

  const prompt = blocks.length
    ? blocks
        .map(
          (b) => `<div class="pblock">
            <div class="c-label">${escapeHtml(titleCase(b.label))}</div>
            <div class="md">${renderMarkdown(b.text)}</div>
          </div>`,
        )
        .join("")
    : `<div class="pblock"><div class="md"><p>${escapeHtml(d.purpose ?? "The request body is not a message list.")}</p></div></div>`;

  const title = escapeHtml(d.prompt_template ?? firstWord(d.purpose));
  const state = parsed ? "parsed" : "unparsed";

  if (compact) {
    const last = blocks.length ? blocks[blocks.length - 1].text : (d.purpose ?? "");
    return `<div class="callparsed compact">
      <h4>${title} · ${state}</h4>
      <div class="c-label">Prompt</div>
      <div class="md clamp">${renderMarkdown(last)}</div>
      <div class="c-label c-gap">Result</div>
      ${table}${candTable}
      ${unparsed}
      ${fieldHtml}
      ${why}
    </div>`;
  }

  return `<div class="callparsed">
    <div class="cgrid">
      <div><div class="c-label">Prompt · rendered</div>${prompt}</div>
      <div><div class="c-label">Result · ${state}</div>${table}${candTable}${unparsed}${fieldHtml}${why}</div>
    </div>
    <details class="craw">
      <summary>Raw request and response</summary>
      <div class="crawgrid">
        <div class="crawcol">
          <div class="c-label">Request <button type="button" class="ccopy">Copy</button></div>
          <pre>${escapeHtml(pretty(d.request_json))}</pre>
        </div>
        <div class="crawcol">
          <div class="c-label">Response <button type="button" class="ccopy">Copy</button></div>
          <pre>${escapeHtml(pretty(d.response_json))}</pre>
        </div>
      </div>
    </details>
  </div>`;
}
