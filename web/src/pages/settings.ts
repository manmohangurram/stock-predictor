import type { Settings } from "../api";
import { renderBalanceRail } from "../components/balance-rail";
import { money } from "../format";

export function renderSettings(s: Settings): string {
  const below = s.budget < s.committed;
  const currencies = ["INR", "USD", "EUR", "GBP"];
  return `<main id="main" class="page">
  <header class="header">
    <h1>Settings</h1>
    <div class="nav">
      <a href="/">Ledger</a><a href="/positions">Positions</a>
      <a href="/settings" aria-current="page">Settings</a><a href="/calls">Audit</a>
    </div>
    <div class="double-rule"></div>
  </header>
  <section class="settings">
    <form id="settings-form">
      <div class="field">
        <label class="label" for="budget">Budget</label>
        <input id="budget" name="budget" type="number" min="0" step="1" value="${Math.round(s.budget)}" data-committed="${s.committed}" />
        <div class="help">Committed is ${money(s.committed, 0)} ${s.currency}. The budget cannot go below it.</div>
        ${below ? `<div class="guard">Committed is ${money(s.committed, 0)} — the budget cannot go below it.</div>` : ""}
      </div>
      <div class="field">
        <label class="label" for="currency">Currency</label>
        <select id="currency" name="currency" ${s.currency_locked ? "disabled" : ""}>
          ${currencies.map((c) => `<option value="${c}" ${c === s.currency ? "selected" : ""}>${c}</option>`).join("")}
        </select>
        ${
          s.currency_locked
            ? `<div class="help">Currency is fixed once the ledger has entries.</div>`
            : `<div class="help">Locked permanently after the first recorded transaction.</div>`
        }
      </div>
      <button class="btn" type="submit" ${below ? "disabled" : ""}>Save</button>
    </form>
  </section>
  ${renderBalanceRail(s)}
</main>`;
}
