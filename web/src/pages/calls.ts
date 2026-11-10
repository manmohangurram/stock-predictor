import type { CallRow, ScheduledPrompt } from "../api";
import { dayTime, escapeHtml, humanLabel, money, titleCase, units } from "../format";
import { renderMarkdown } from "../markdown";
import { renderApp, renderHead } from "../components/shell";
import { actionStamp, pill, stamp, toneOf } from "../components/stamp";
import {
  declaredTools,
  parseReply,
  pretty,
  promptBlocks,
  replyText,
  searchQuery,
  searchResults,
  type CallDetail,
} from "../components/call-viewer";

export type { CallDetail };


/** What you did with the decision this call produced — or why it never reached you. */
function reviewBadge(r: CallRow): string {
  if (r.error) {
    // A call that broke is not a decision at all: say so here, and the message is in View.
    return pill("failed", "failed", escapeHtml(r.error));
  }
  if (r.decision_action === "BLOCKED") {
    // A refused plan is the model's answer, recorded with its reason in View.
    // The reason is long; it lives in View, so the row stays one line.
    return pill("todo", "blocked", escapeHtml(r.decision_reason ?? ""));
  }
  if (r.decision_action === "PLAN") {
    const state = r.decision_status ?? "pending";
    const label = state === "pending" ? "plan · awaiting you" : `plan · ${state}`;
    return pill(state === "approved" ? "ok" : state === "rejected" ? "todo" : "", label);
  }
  if (!r.decision_status) return "";
  const tone = r.decision_status === "approved" ? "ok" : r.decision_status === "rejected" ? "todo" : "";
  const label =
    r.decision_status === "pending" ? "awaiting you" : r.decision_status;
  return pill(
    tone as "ok" | "todo" | "",
    label,
    r.decision_id ? `decision #${r.decision_id}` : "",
  );
}

/**
 * Where you are in the record, and how much of it you want: offset/limit in the URL, so a page
 * survives a refresh and a link can point at it.
 */
export function renderPager(offset: number, limit: number, total: number): string {
  const pages = Math.max(1, Math.ceil(total / limit));
  const page = Math.floor(offset / limit) + 1;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + limit, total);
  const link = (next: number, label: string, on: boolean) =>
    on
      ? `<a class="btn ghost" href="/calls?offset=${next}&limit=${limit}" data-offset="${next}">${label}</a>`
      : `<span class="btn ghost" aria-disabled="true">${label}</span>`;
  const sizes = [25, 50, 100]
    .map(
      (n) => `<option value="${n}" ${n === limit ? "selected" : ""}>${n} per page</option>`,
    )
    .join("");
  return `<nav class="pager" aria-label="Calls">
    <div class="pager-side">
      ${link(Math.max(0, offset - limit), "‹ Newer", offset > 0)}
      ${link(offset + limit, "Older ›", offset + limit < total)}
    </div>
    <span class="pager-note">${from}–${to} of ${total} · page ${page} of ${pages}</span>
    <label class="pager-size"><span class="lbl">Show</span>
      <select data-page-size aria-label="Calls per page">${sizes}</select></label>
  </nav>`;
}

/**
 * The schedule, one card per prompt: when it last ran and how that went, when it is due next, and
 * a way to start it now. The cadence is printed in words because "next due 09:15" means nothing
 * without "every tick, 09:00–16:00 IST, Mon–Fri".
 */
export function renderSchedule(
  prompts: ScheduledPrompt[],
  now: string,
  running: number,
): string {
  const due = (iso: string | null) => {
    if (!iso) return "—";
    return Date.parse(iso) <= Date.parse(now) + 1000 ? "due now" : dayTime(iso);
  };
  const took = (r: NonNullable<ScheduledPrompt["last"]>) => {
    if (!r.finished_at) return "running";
    const ms = Date.parse(r.finished_at) - Date.parse(r.at);
    if (!Number.isFinite(ms) || ms < 0) return "—";
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
  };

  const card = (p: ScheduledPrompt) => {
    const r = p.last;
    const state = !r
      ? pill("", "never run")
      : r.status === "running"
        ? pill("running", "running")
        : r.status === "failed"
          ? pill("failed", "failed")
          : pill("done", "done");
    const when = r ? `last ${dayTime(r.at)}` : "no run yet";
    return `<article class="rcard" data-state="${r?.status ?? "none"}">
      <div class="rcard-top">${state}<span class="run-when">${when}</span></div>
      <div class="rcard-title">${escapeHtml(p.label)}
        ${r ? `<span class="trigger">${escapeHtml(r.trigger)}${r.finished_at ? ` · ${took(r)}` : ""}</span>` : ""}</div>
      <div class="rcard-result">${escapeHtml(
        r?.error ?? r?.summary ?? (r ? "—" : "Nothing has run yet."),
      )}</div>
      <div class="rcard-next"><span class="lbl">Next due</span> <b>${escapeHtml(due(p.next_due_at))}</b></div>
      <div class="rcard-rule">${escapeHtml(p.rule)}</div>
      ${
        r?.status === "running"
          ? `<button class="mini" type="button" disabled>Working…</button>`
          : `<button class="mini" type="button" data-run-again="${escapeHtml(p.prompt)}">Run again</button>`
      }
    </article>`;
  };

  const choice = (value: string, text: string) => `<option value="${value}">${text}</option>`;
  return `<div class="runs-strip">
    ${prompts.map(card).join("")}
    <article class="rcard rcard-add">
      <div class="rcard-title">Run now</div>
      <form class="run-now" data-run-now>
        <select name="prompt" aria-label="What to run">
          ${choice("plan_stock", "Stock plan")}
          ${choice("plan_fund", "Fund plan")}
          ${choice("short", "Short-term pass")}
          ${choice("poll", "A poll")}
          ${choice("decide", "One instrument")}
        </select>
        <input name="symbol" placeholder="INFY.NS" aria-label="Symbol" />
        <button class="btn primary" type="submit">Run</button>
        <span class="help" data-run-status role="status">${
          running ? `${running} running now` : ""
        }</span>
      </form>
    </article>
  </div>`;
}

export interface CallFilters {
  prompt?: string;
  /**
   * The exchange channel — named `channel` in the URL because `?call=` already means
   * "open this call", and a filter that silently swallowed the deep link emptied the table.
   */
  channel?: string;
  model?: string;
  review?: string;
}

/** The filter row: four dimensions, all server-side, all in the URL. */
function filterBar(
  facets: { prompts: string[]; calls: string[]; models: string[] },
  active: CallFilters,
): string {
  const pick = (name: keyof CallFilters, label: string, values: string[]) => {
    const options = values
      .map(
        (v) =>
          `<option value="${escapeHtml(v)}" ${active[name] === v ? "selected" : ""}>${escapeHtml(humanLabel(v))}</option>`,
      )
      .join("");
    return `<label class="filter"><span class="lbl">${label}</span>
      <select data-filter="${name}" aria-label="${label}">
        <option value="">Any</option>${options}
      </select></label>`;
  };
  const on = Object.values(active).some(Boolean);
  return `<div class="filter-bar">
    ${pick("prompt", "Prompt", facets.prompts)}
    ${pick("channel", "Call", facets.calls)}
    ${pick("model", "Model", facets.models)}
    ${pick("review", "Review", ["pending", "approved", "rejected", "blocked", "none"])}
    ${on ? `<a class="btn ghost" href="/calls">Clear</a>` : ""}
  </div>`;
}

export function renderCalls(
  schedule: { prompts: ScheduledPrompt[]; now: string },
  running: number,
  rows: CallRow[],
  facets: { prompts: string[]; calls: string[]; models: string[] },
  active: CallFilters,
  offset = 0,
  limit = 25,
  total = rows.length,
): string {
  const failures = rows.filter((r) => r.error).length;
  const tokens = rows.reduce(
    (n, r) => n + (r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0),
    0,
  );
  const body = rows
    .map((r) => {
      // What the call was: a prompt for a model, or a tool call. A search's purpose is the query,
      // so its first word is not a prompt type — "MARUTI news today" is not a call named MARUTI.
      const tool = r.endpoint === "search" || r.endpoint === "fetch";
      const [kind, ...rest] = (r.purpose ?? "").split(/\s+/);
      const promptType = tool
        ? r.endpoint === "search"
          ? "Web search"
          : "Web fetch"
        : titleCase(kind || "—");
      // A book call is about no single holding: say so, instead of printing "book".
      const holding =
        !tool && kind === "plan"
          ? `<b>Whole book</b><div class="sub">every holding</div>`
          : r.instrument_name
            ? `<b>${escapeHtml(r.instrument_name)}</b>${
                r.instrument_symbol
                  ? `<div class="sub">${escapeHtml(r.instrument_symbol)}</div>`
                  : ""
              }`
            : tool
              ? // A tool call's purpose is the query itself ("MARUTI news today"), which is the
                // most a row can say when the holding is not in the book yet.
                `<span class="muted">${escapeHtml(r.purpose ?? "—")}</span>`
              : `<span class="muted">${escapeHtml(rest.join(" ") || "—")}</span>`;
      return `<tr>
  <td class="time" title="${escapeHtml(r.ts)}">${escapeHtml(dayTime(r.ts))}</td>
  <td class="hide-sm">${escapeHtml(promptType)}</td>
  <td class="hide-sm">${escapeHtml(titleCase(r.endpoint))}</td>
  <td>${holding}</td>
  <td class="review-td"><div class="review-cell">${reviewBadge(r)}</div></td>
  <td class="right">
    <button class="btn ghost view-call" type="button" data-id="${r.id}"
            aria-expanded="false" aria-controls="call-${r.id}" aria-label="Open the call">Open</button>
  </td>
</tr>
<tr class="call-detail" id="call-${r.id}" hidden><td colspan="6"></td></tr>`;
    })
    .join("");

  return renderApp("audit", `${renderHead("Audit", "Every outbound model call, newest first", [
      { label: "Calls", value: String(rows.length) },
      { label: "Failures", value: String(failures), tone: failures ? "neg" : undefined },
      { label: "Tokens", value: tokens.toLocaleString("en-IN") },
    ])}
  ${renderSchedule(schedule.prompts, schedule.now, running)}
  ${filterBar(facets, active)}
  <section class="main-body table-wrap">
    <table class="blotter">
      <thead><tr>
        <th scope="col">Time</th><th scope="col" class="hide-sm">Prompt type</th>
        <th scope="col" class="hide-sm">Call</th>
        <th scope="col">Holding</th><th scope="col">Review</th>
        <th scope="col" class="right"><span class="hide-sm">Open</span></th>
      </tr></thead>
      <tbody>${body || `<tr><td colspan="6" class="empty">No calls yet.</td></tr>`}</tbody>
    </table>
    ${renderPager(offset, limit, total)}
  </section>`);
}

/** The reply as a decision, not as JSON: a verdict, its numbers, and what it wants next. */
/** One answer card, always the same shape: a verdict head, its figures or moves, then its words. */
function decisionCard(reply: string, when: string): string {
  const r = parseReply(reply);
  const head = (label: string, tone: string, title: string) =>
    `<div class="answer-head">
      ${stamp(tone as "buy" | "sell" | "estimate", label)}
      <span class="answer-title">${title}</span>
      <span class="answer-when">${escapeHtml(when)}</span>
    </div>`;

  // A plan: one stamped row per move, then why it was made.
  if (r.moves.length) {
    const legs = r.moves
      .map((m) => {
        const buy = m.action === "BUY";
        return `<tr>
          <td>${actionStamp(m.action)}</td>
          <td class="answer-sym">${escapeHtml(m.symbol)}${
            m.why ? `<div class="answer-why">${escapeHtml(m.why)}</div>` : ""
          }</td>
          <td class="num">${m.qty === null ? "—" : units(m.qty)}</td>
          <td class="num">${m.price === null ? "—" : money(m.price)}</td>
          <td class="num">${m.stop === null ? "—" : money(m.stop)}</td>
          <td class="num">${m.target === null ? "—" : money(m.target)}</td>
        </tr>`;
      })
      .join("");
    return `<article class="answer" data-verdict="plan">
      ${head("PLAN", "estimate", `${r.moves.length} move${r.moves.length === 1 ? "" : "s"}`)}
      <div class="answer-body">
        <table class="answer-moves">
          <thead><tr><th></th><th>Holding</th><th class="num">Qty</th><th class="num">Price</th>
            <th class="num">Stop</th><th class="num">Target</th></tr></thead>
          <tbody>${legs}</tbody>
        </table>
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

  // Only what the row above cannot say. The type, the channel, the holding and the verdict are
  // already on the row; repeating them here was the duplication.
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
          : decisionCard(reply, dayTime(d.ts))
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
