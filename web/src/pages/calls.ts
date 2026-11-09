import type { CallRow } from "../api";
import { escapeHtml, money, units } from "../format";
import { renderApp, renderHead } from "../components/shell";
import {
  declaredTools,
  parseReply,
  pretty,
  promptBlocks,
  replyText,
  type CallDetail,
} from "../components/call-viewer";

export type { CallDetail };

/** The browser's own clock: what a person sees, not the UTC the server stores. */
function localTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  // 24-hour and short, so the column stays one line: "09 Nov 05:34".
  const day = d.toLocaleDateString("en-IN", { day: "2-digit", month: "short" });
  const time = d.toLocaleTimeString("en-IN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${day} ${time}`;
}

/** What you did with the decision this call produced — or why it never reached you. */
function reviewBadge(r: CallRow): string {
  if (r.decision_action === "BLOCKED") {
    // A refused plan is the model's answer, recorded with its reason in View.
    // The reason is long; it lives in View, so the row stays one line.
    return `<span class="pill todo" title="${escapeHtml(r.decision_reason ?? "")}">blocked</span>`;
  }
  if (r.decision_action === "PLAN") {
    const state = r.decision_status ?? "pending";
    const label = state === "pending" ? "plan · awaiting you" : `plan · ${state}`;
    return `<span class="pill ${state === "approved" ? "ok" : state === "rejected" ? "todo" : ""}">${label}</span>`;
  }
  if (!r.decision_status) return `<span class="muted">no decision</span>`;
  const tone = r.decision_status === "approved" ? "ok" : r.decision_status === "rejected" ? "todo" : "";
  const label =
    r.decision_status === "pending" ? "awaiting you" : r.decision_status;
  return `<span class="pill ${tone}" ${r.decision_id ? `title="decision #${r.decision_id}"` : ""}>${label}</span>`;
}

export function renderCalls(rows: CallRow[]): string {
  const failures = rows.filter((r) => r.error).length;
  const tokens = rows.reduce(
    (n, r) => n + (r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0),
    0,
  );
  const body = rows
    .map((r) => {
      // "decide INFY.NS" is two facts: what the call was for, and which holding it was about.
      const [kind, ...rest] = (r.purpose ?? "").split(/\s+/);
      // A book call is about no single holding: say so, instead of printing "book".
      const holding =
        kind === "plan"
          ? `<b>Whole book</b><div class="sub">every holding</div>`
          : r.instrument_name
            ? `<b>${escapeHtml(r.instrument_name)}</b>${
                r.instrument_symbol
                  ? `<div class="sub">${escapeHtml(r.instrument_symbol)}</div>`
                  : ""
              }`
            : `<span class="muted">${escapeHtml(rest.join(" ") || "—")}</span>`;
      return `<tr>
  <td class="time num" title="${escapeHtml(r.ts)}">${escapeHtml(localTime(r.ts))}</td>
  <td>${escapeHtml(kind || "—")}</td>
  <td class="hide-sm">${escapeHtml(r.endpoint)}</td>
  <td>${holding}</td>
  <td class="review-td"><div class="review-cell">${reviewBadge(r)}</div></td>
  <td class="hide-sm ${r.error ? "negative" : ""}">${escapeHtml(r.error ?? r.validator_verdict ?? "ok")}</td>
  <td class="right">
    <button class="btn ghost view-call" type="button" data-id="${r.id}"
            aria-expanded="false" aria-controls="call-${r.id}" aria-label="View the call">View</button>
  </td>
</tr>
<tr class="call-detail" id="call-${r.id}" hidden><td colspan="7"></td></tr>`;
    })
    .join("");

  return renderApp("audit", `${renderHead("Audit", "Every outbound model call, newest first", [
      { label: "Calls", value: String(rows.length) },
      { label: "Failures", value: String(failures), tone: failures ? "neg" : undefined },
      { label: "Tokens", value: tokens.toLocaleString("en-IN") },
    ])}
  <p class="caption-line">The prompt and the reply are stored with every call — open one to read them</p>
  <section class="main-body table-wrap">
    <table class="blotter">
      <thead><tr>
        <th scope="col">Time</th><th scope="col">Prompt type</th>
        <th scope="col" class="hide-sm">Call</th>
        <th scope="col">Holding</th><th scope="col">Review</th>
        <th scope="col" class="hide-sm">Outcome</th>
        <th scope="col" class="right"><span class="hide-sm">View</span></th>
      </tr></thead>
      <tbody>${body || `<tr><td colspan="7" class="empty">No calls yet.</td></tr>`}</tbody>
    </table>
  </section>`);
}

/** The reply as a decision, not as JSON: a verdict, its numbers, and what it wants next. */
/** One answer card, always the same shape: a verdict head, its figures or moves, then its words. */
function decisionCard(reply: string, when: string): string {
  const r = parseReply(reply);
  const head = (stamp: string, tone: string, title: string) =>
    `<div class="answer-head">
      <span class="stamp" data-stamp="${tone}">${escapeHtml(stamp)}</span>
      <span class="answer-title">${title}</span>
      <span class="answer-when">${escapeHtml(when)}</span>
    </div>`;

  // A plan: one stamped row per move, then why it was made.
  if (r.moves.length) {
    const legs = r.moves
      .map((m) => {
        const buy = m.action === "BUY";
        return `<tr>
          <td><span class="stamp" data-stamp="${buy ? "buy" : "sell"}">${escapeHtml(m.action)}</span></td>
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
      ${head("STOCK PLAN", "estimate", `${r.moves.length} move${r.moves.length === 1 ? "" : "s"}`)}
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

export function renderExchangeDetail(d: CallDetail): string {
  const blocks = promptBlocks(d.request_json);
  const reply = replyText(d.response_json);
  const tools = declaredTools(d.request_json);
  const prompt = blocks.length
    ? blocks
        .map(
          (b) => `<section class="call-block">
      <div class="lbl">${escapeHtml(b.label)}</div>
      <pre>${escapeHtml(b.text)}</pre>
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
    ${fact("Model", escapeHtml(d.prompt_template ?? firstWord(d.purpose)))}
    ${fact("Time", escapeHtml(localTime(d.ts)))}
    ${d.round ? fact("Round", String(d.round)) : ""}
    ${fact("Tokens", `${d.prompt_tokens ?? 0} in · ${d.completion_tokens ?? 0} out`)}
    ${tools.length ? fact("Tools", escapeHtml(tools.join(", "))) : ""}
    ${d.trigger ? `<span class="call-fact why"><span class="lbl">Asked because</span><b>${escapeHtml(d.trigger)}</b></span>` : ""}
    ${d.decision_reason ? `<span class="call-fact refused"><span class="lbl">Refused</span><b>${escapeHtml(d.decision_reason)}</b></span>` : ""}
    ${d.error ? `<span class="call-fact refused"><span class="lbl">Failed</span><b>${escapeHtml(d.error)}</b></span>` : ""}
  </div>

  ${
    d.error
      ? ""
      : `<section class="call-sec">
    <h3 class="call-h">The answer</h3>
    ${decisionCard(reply, localTime(d.ts))}
  </section>`
  }

  <section class="call-sec call-exchange">
    <div class="call-col">
      <h3 class="call-h">Sent to the model</h3>
      ${prompt}
    </div>
    <div class="call-col">
      <h3 class="call-h">Received, verbatim</h3>
      <pre class="call-verbatim">${reply ? escapeHtml(reply) : "no content"}</pre>
    </div>
  </section>

  <details class="raw">
    <summary>Raw request and response</summary>
    <div class="call-exchange">
      <section class="call-block"><h3 class="call-h">request_json</h3><pre>${escapeHtml(pretty(d.request_json))}</pre></section>
      <section class="call-block"><h3 class="call-h">response_json</h3><pre>${escapeHtml(pretty(d.response_json))}</pre></section>
    </div>
  </details>
</div>`;
}
