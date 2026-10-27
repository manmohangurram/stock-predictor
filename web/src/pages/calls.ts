import type { CallRow } from "../api";
import { escapeHtml } from "../format";

export function renderCalls(rows: CallRow[]): string {
  const body = rows
    .map(
      (r) => `<tr>
  <td class="time num">${escapeHtml(r.ts.slice(11, 16))}</td>
  <td>${escapeHtml(r.endpoint)}</td>
  <td>${escapeHtml(r.purpose ?? "")}</td>
  <td class="num right">${r.round ?? "—"}</td>
  <td class="num right">${r.prompt_tokens ?? 0}/${r.completion_tokens ?? 0}</td>
  <td class="${r.error ? "negative" : ""}">${escapeHtml(r.error ?? r.validator_verdict ?? "ok")}</td>
</tr>`,
    )
    .join("");
  return `<main id="main" class="page">
  <header class="header">
    <h1>Audit</h1>
    <div class="nav">
      <a href="/">Ledger</a><a href="/positions">Positions</a>
      <a href="/settings">Settings</a><a href="/calls" aria-current="page">Audit</a>
    </div>
    <div class="double-rule"></div>
  </header>
  <section>
    <table class="ledger">
      <caption>Every outbound model call, newest first</caption>
      <thead><tr>
        <th scope="col">Time</th><th scope="col">Call</th><th scope="col">Purpose</th>
        <th scope="col" class="right">Round</th><th scope="col" class="right">Tokens</th>
        <th scope="col">Outcome</th>
      </tr></thead>
      <tbody>${body || `<tr><td colspan="6" class="empty">No calls yet.</td></tr>`}</tbody>
    </table>
  </section>
</main>`;
}
