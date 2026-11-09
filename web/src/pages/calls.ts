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
    // The reason is long; it lives in View, so the row stays one line.
    return `<span class="pill todo" title="${escapeHtml(r.decision_reason ?? "")}">blocked</span>`;
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
      const holding = r.instrument_name
        ? `<b>${escapeHtml(r.instrument_name)}</b>${
            r.instrument_symbol ? `<div class="sub">${escapeHtml(r.instrument_symbol)}</div>` : ""
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

function firstWord(text: string | null): string {
  return (text ?? "").split(/\s+/)[0] || "—";
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

  const fact = (label: string, value: string) =>
    `<div class="fact"><span class="lbl">${label}</span><b>${value}</b></div>`;
  const holding = d.instrument_name
    ? `<b>${escapeHtml(d.instrument_name)}</b>${
        d.instrument_symbol ? ` <span class="sub">${escapeHtml(d.instrument_symbol)}</span>` : ""
      }`
    : escapeHtml(d.purpose ?? "—");

  return `<div class="call">
  <div class="call-facts">
    ${fact("Prompt type", d.prompt_template ? `<span class="pill ok">${escapeHtml(d.prompt_template)}</span>` : escapeHtml(firstWord(d.purpose)))}
    ${fact("Call", escapeHtml(d.endpoint))}
    ${fact("Holding", holding)}
    ${d.round ? fact("Round", String(d.round)) : ""}
    ${fact("Tokens", `${d.prompt_tokens ?? 0} in · ${d.completion_tokens ?? 0} out`)}
    ${fact("Outcome", escapeHtml(d.error ?? d.validator_verdict ?? "ok"))}
  </div>
  ${
    d.trigger
      ? `<p class="why-line"><span class="lbl">Why it was asked</span> ${escapeHtml(d.trigger)}</p>`
      : ""
  }
  ${
    d.decision_reason
      ? `<p class="why-line refused"><span class="lbl">Refused</span> ${escapeHtml(d.decision_reason)}</p>`
      : ""
  }
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
