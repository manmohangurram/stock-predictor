import type { Dashboard, SignalRow } from "../api";
import { renderBalanceRail } from "../components/balance-rail";
import { renderLedgerRow, type SignalView } from "../components/ledger-row";

const TABLE_HEAD = `<thead>
    <tr>
      <th scope="col">Time</th>
      <th scope="col">Instrument</th>
      <th scope="col">Action</th>
      <th scope="col" class="right">Qty</th>
      <th scope="col" class="right">Entry</th>
      <th scope="col" class="right">Stop</th>
      <th scope="col" class="right">Balance</th>
    </tr>
  </thead>`;

const COLS = `<colgroup>
    <col style="width:56px"><col><col style="width:92px">
    <col style="width:64px"><col style="width:92px"><col style="width:92px">
    <col style="width:100px">
  </colgroup>`;

/** Balance runs down the page: each actionable row shows what the pot becomes after it. */
function withBalances(rows: SignalRow[], available: number): SignalView[] {
  let running = available;
  return rows.map((s) => {
    const value = (s.entry ?? 0) * (s.qty ?? 0);
    if (s.action === "BUY") running -= value;
    if (s.action === "SELL") running += value;
    return { ...s, balanceAfter: running };
  });
}

export function renderLedger(dashboard: Dashboard): string {
  const actionable = withBalances(dashboard.signals, dashboard.available);
  const blocked = dashboard.blocked;

  const holdNote =
    actionable.length === 0 && blocked.length === 0
      ? `<tr><td colspan="7" class="empty">No entries today. Market closes 15:30.</td></tr>` +
        `<tr><td colspan="7" class="ledger-blank"></td></tr>`.repeat(3)
      : actionable.map(renderLedgerRow).join("");

  const blockedTable = blocked.length
    ? `<table class="ledger blocked-table">
        <caption>Not actionable — the budget refused these</caption>
        ${COLS}${TABLE_HEAD}
        <tbody>${blocked.map(renderLedgerRow).join("")}</tbody>
      </table>`
    : "";

  const committedByClass = {
    Stocks: dashboard.positions.filter((p) => p.class === "stock").reduce((a, p) => a + p.qty * p.avg_cost, 0),
    "Mutual funds": dashboard.positions.filter((p) => p.class === "mutual_fund").reduce((a, p) => a + p.qty * p.avg_cost, 0),
    "IPOs reserved":
      dashboard.committed -
      dashboard.positions.filter((p) => p.class !== "ipo").reduce((a, p) => a + p.qty * p.avg_cost, 0),
  };

  return `<main id="main" class="page">
  <header class="header">
    <h1>Ledger</h1>
    <div class="nav">
      <a href="/" aria-current="page">Ledger</a>
      <a href="/positions">Positions</a>
      <a href="/settings">Settings</a>
      <a href="/calls">Audit</a>
    </div>
    <div class="double-rule"></div>
  </header>
  <div class="ledger-col">
    <table class="ledger">
      <caption>Today's entries, newest first</caption>
      ${COLS}${TABLE_HEAD}
      <tbody>${holdNote}</tbody>
    </table>
    ${blockedTable}
  </div>
  ${renderBalanceRail(dashboard, committedByClass)}
</main>`;
}
