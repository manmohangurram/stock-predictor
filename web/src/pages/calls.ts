import type { CallRow } from "../api";
import { escapeHtml, money } from "../format";
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

export function renderCalls(rows: CallRow[]): string {
  const failures = rows.filter((r) => r.error).length;
  const tokens = rows.reduce(
    (n, r) => n + (r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0),
    0,
  );
  const body = rows
    .map(
      (r) => `<tr>
  <td class="time num">${escapeHtml(r.ts.slice(11, 16))}</td>
  <td>${escapeHtml(r.endpoint)}</td>
  <td>${escapeHtml(r.purpose ?? "")}</td>
  <td class="num right">${r.round ?? "—"}</td>
  <td class="num right">${r.prompt_tokens ?? 0}/${r.completion_tokens ?? 0}</td>
  <td class="${r.error ? "negative" : ""}">${escapeHtml(r.error ?? r.validator_verdict ?? "ok")}</td>
  <td class="right">
    <button class="btn ghost view-call" type="button" data-id="${r.id}"
            aria-expanded="false" aria-controls="call-${r.id}">View</button>
  </td>
</tr>
<tr class="call-detail" id="call-${r.id}" hidden><td colspan="7"></td></tr>`,
    )
    .join("");

  return renderApp("audit", `${renderHead("Audit", "Every outbound model call, newest first", [
      { label: "Calls", value: String(rows.length) },
      { label: "Failures", value: String(failures), tone: failures ? "neg" : undefined },
      { label: "Tokens", value: tokens.toLocaleString("en-IN") },
    ])}
  <section class="main-body">
    <table class="blotter">
      <caption>The prompt and the reply are stored with every call — open one to read them</caption>
      <thead><tr>
        <th scope="col">Time</th><th scope="col">Call</th><th scope="col">Purpose</th>
        <th scope="col" class="right">Round</th><th scope="col" class="right">Prompt/Reply</th>
        <th scope="col">Outcome</th><th scope="col" class="right"><span class="hide-sm">Exchange</span></th>
      </tr></thead>
      <tbody>${body || `<tr><td colspan="7" class="empty">No calls yet.</td></tr>`}</tbody>
    </table>
  </section>`);
}

/** The reply as a decision, not as JSON: a verdict, its numbers, and what it wants next. */
function decisionCard(reply: string): string {
  const r = parseReply(reply);
  if (!r.verdict) {
    return `<section class="call-block"><pre>${
      reply ? escapeHtml(reply) : "<em>no readable content — see the raw JSON</em>"
    }</pre></section>`;
  }
  const stamp = r.verdict === "BUY" ? "buy" : r.verdict === "SELL" ? "sell" : "estimate";
  const row = (label: string, value: string) =>
    `<div class="reply-figure"><span class="lbl">${label}</span><b class="num">${value}</b></div>`;
  const figures = [
    r.qty !== null ? row("Quantity", String(r.qty)) : "",
    r.entry !== null ? row("Entry", money(r.entry)) : "",
    r.stop !== null ? row("Stop", money(r.stop)) : "",
    r.target !== null ? row("Target", money(r.target)) : "",
    r.confidence ? row("Confidence", r.confidence) : "",
  ].join("");
  const wants = r.wantsHeadlines.length
    ? `<p class="decision-note">Asked for headline ${r.wantsHeadlines.join(", ")}${
        r.wantsHistory ? " and the 7-day history" : ""
      } before deciding.</p>`
    : "";
  const prose = [r.rationale, r.why].filter(Boolean).join("\n\n");
  const cites = r.cites.length
    ? `<div class="decision-cites">${r.cites
        .map((c) => `<span class="chip">source ${c}</span>`)
        .join("")}</div>`
    : "";
  return `<article class="decision" data-verdict="${r.verdict.toLowerCase()}">
    <div class="decision-head">
      <span class="stamp" data-stamp="${stamp}">${r.verdict}</span>
      <span class="sub">the model's answer, parsed</span>
    </div>
    ${figures ? `<div class="decision-grid">${figures}</div>` : ""}
    ${prose ? `<p class="decision-prose">${escapeHtml(prose)}</p>` : ""}
    ${wants}
    ${cites}
  </article>`;
}

/** The expanded exchange: prompt by role, the reply, then the raw bodies on demand. */
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
    : `<p class="empty">The request body is not a message list — see the raw JSON.</p>`;

  return `<div class="call">
  <div class="call-meta">
    <span class="lbl">${escapeHtml(d.endpoint)}${d.round ? ` · round ${d.round}` : ""}</span>
    ${d.purpose ? `<span class="mono">${escapeHtml(d.purpose)}</span>` : ""}
    ${d.prompt_template ? `<span class="pill ok">${escapeHtml(d.prompt_template)}</span>` : ""}
    <span class="mono">${d.prompt_tokens ?? 0} prompt tokens</span>
    ${tools.length ? `<span class="mono">tools: ${escapeHtml(tools.join(", "))}</span>` : ""}
    ${d.error ? `<span class="pill todo">${escapeHtml(d.error)}</span>` : ""}
    ${d.validator_verdict ? `<span class="mono">${escapeHtml(d.validator_verdict)}</span>` : ""}
  </div>
  <div class="call-grid">
    <div class="call-col">
      <h3 class="lbl">What the model was sent</h3>
      ${prompt}
    </div>
    <div class="call-col">
      <h3 class="lbl">What it replied</h3>
      ${decisionCard(reply)}
    </div>
  </div>
  <details class="raw">
    <summary>Raw request and response</summary>
    <div class="call-grid">
      <section class="call-block"><h3 class="lbl">request_json</h3><pre>${escapeHtml(pretty(d.request_json))}</pre></section>
      <section class="call-block"><h3 class="lbl">response_json</h3><pre>${escapeHtml(pretty(d.response_json))}</pre></section>
    </div>
  </details>
</div>`;
}
