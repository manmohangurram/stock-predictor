import type { SignalRow } from "../api";
import { escapeHtml, money, units } from "../format";
import { today } from "../pages/instrument";

/**
 * Approve and reject, wherever a pending decision is shown — the ledger and the instrument page
 * use this one markup, so the behaviour cannot drift between them.
 *
 * Approve opens the fill pre-filled from the decision; the quantity and price are editable because
 * you rarely get the exact price a proposal named.
 */
export function renderDecisionActions(s: SignalRow): string {
  if (s.status !== "pending" || !["BUY", "SELL"].includes(s.action)) return "";
  return `<div class="decision-actions">
    <button class="btn icon-btn" type="button" data-approve="${s.id}" aria-expanded="false"
            title="Approve — record the fill you actually got" aria-label="Approve">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3.4 8.4l3 3 7-7" /></svg>
    </button>
    <button class="btn ghost icon-btn" type="button" data-reject="${s.id}" aria-expanded="false"
            title="Reject — with a reason, no money moves" aria-label="Reject">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M4 4l8 8M12 4l-8 8" /></svg>
    </button>
  </div>
  <form class="approve-form" data-approve-form="${s.id}" hidden>
    <div class="fill-grid">
      <label class="field"><span class="lbl">Quantity</span>
        <input name="qty" type="number" min="0" step="any" value="${s.qty ?? ""}" /></label>
      <label class="field"><span class="lbl">Price</span>
        <input name="price" type="number" min="0" step="any" value="${s.entry ?? ""}" /></label>
      <label class="field"><span class="lbl">Date</span>
        <input name="at" type="date" value="${today()}" max="${today()}" /></label>
      <label class="field"><span class="lbl">Proposed</span>
        <span class="static">${s.qty === null ? "—" : units(s.qty)} @ ${s.entry === null ? "—" : money(s.entry)}</span></label>
    </div>
    <div class="row">
      <button class="btn primary" type="submit">Post the fill</button>
      <button class="btn ghost" type="button" data-cancel="${s.id}">Cancel</button>
      <span class="help" data-status="${s.id}" role="status"></span>
    </div>
  </form>
  <form class="reject-form" data-reject-form="${s.id}" hidden>
    <div class="row">
      <input name="reason" placeholder="Why not? (optional)" aria-label="Reason"
             value="${escapeHtml(s.status_reason ?? "")}" />
      <button class="btn" type="submit">Reject</button>
      <button class="btn ghost" type="button" data-cancel="${s.id}">Cancel</button>
      <span class="help" data-status="${s.id}" role="status"></span>
    </div>
  </form>`;
}
