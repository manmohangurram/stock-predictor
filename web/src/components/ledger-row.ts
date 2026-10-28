import type { SignalRow } from "../api";
import { clock, day, escapeHtml, money, units } from "../format";
import { stamp } from "./stamp";

export type SignalView = SignalRow & { balanceAfter?: number };

const ESTIMATE_CLASSES = ["mutual_fund", "ipo"];

/** MF/IPO timing copy: a plan, never a firm price. Static literals, so not escaped. */
export function timingText(s: SignalRow): string {
  if (s.class === "mutual_fund") {
    return s.action === "SELL"
      ? "NAV at today's 15:00 cutoff · cash arrives T+2"
      : "units at today's 15:00 NAV cutoff";
  }
  if (s.class === "ipo") return "cash locked until allotment · not sellable until listing";
  return "";
}

export function renderLedgerRow(s: SignalView): string {
  const isBlocked = s.action === "BLOCKED";
  const isEstimate = ESTIMATE_CLASSES.includes(s.class);
  const blockedAttr = isBlocked ? " data-blocked" : "";

  const actionStamp = isBlocked
    ? stamp("blocked", "blocked")
    : isEstimate
      ? stamp("estimate", "estimate")
      : stamp(s.action === "SELL" ? "sell" : "buy", s.action);

  const price = s.entry === null ? "—" : money(s.entry);
  const stop = s.stop === null ? "—" : money(s.stop);
  const target = s.target === null ? null : money(s.target);
  const qty = s.qty === null || s.qty === undefined ? "—" : units(s.qty);
  const balance = s.balanceAfter === undefined ? "" : money(s.balanceAfter, 0);
  const shortfall = s.reasons.find((r) => r.startsWith("short by"));

  return `<tr class="row-main${isBlocked ? " blocked-row" : ""}"${blockedAttr}>
  <td class="time num">${clock(s.created_at)}</td>
  <td class="identity">
    <a class="symbol" href="/i/${encodeURIComponent(s.symbol)}">${escapeHtml(s.symbol)}</a>
    <span class="class-chip">${s.class.replace("_", " ")}</span>
    ${isEstimate ? `<span class="estimate">${timingText(s)}</span>` : ""}
  </td>
  <td>${actionStamp}</td>
  <td class="num right" data-label="Qty">${qty}</td>
  <td class="num right ${isEstimate ? "estimate" : ""}" data-label="Entry">${price}</td>
  <td class="num right" data-label="Stop">${stop}</td>
  <td class="num right balance" data-label="Balance">${balance}</td>
</tr>
<tr class="detail-row" hidden><td colspan="7">
  ${
    isBlocked
      ? `<p class="blocked-note">insufficient cash — ${escapeHtml(shortfall ?? "short of the budget")}</p>`
      : `<ul>${s.reasons.map((r) => `<li>${escapeHtml(r)}</li>`).join("")}</ul>`
  }
  ${s.rationale ? `<p class="rationale">${escapeHtml(s.rationale)}</p>` : ""}
  ${target ? `<p class="cites">target ${target}</p>` : ""}
  <p class="cites">valid until ${escapeHtml(day(s.valid_until))} · ${timingText(s) || escapeHtml(s.timing_note)}</p>
</td></tr>`;
}
