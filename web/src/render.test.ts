import { describe, expect, test } from "vitest";
import type { Dashboard, Settings, SignalRow } from "./api";
import { renderLedgerRow } from "./components/ledger-row";
import { renderLedger } from "./pages/ledger";
import { renderSettings } from "./pages/settings";

const base = {
  entry: 1405.2,
  stop: 1412.0,
  target: 1510.0,
  valid_until: "2026-10-28T10:00:00Z",
  timing_note: "limit order, intraday or next open",
  reasons: ["close 1405.20 below recorded stop 1412.00"],
  rationale: "guidance raised with margin expansion",
  created_at: "2026-10-27T04:11:00Z",
};

const stockSell: SignalRow = {
  ...base,
  id: 1,
  qty: 1,
  symbol: "INFY",
  class: "stock",
  action: "SELL",
};

describe("ledger row", () => {
  test("renders the running balance after the action", () => {
    const html = renderLedgerRow({ ...stockSell, balanceAfter: 84900 });
    expect(html).toContain("1,405.20");
    expect(html).toContain("84,900");
    expect(html).toContain('data-stamp="sell"');
  });

  test("estimate rows are labelled and never presented as a firm sell", () => {
    const html = renderLedgerRow({
      ...stockSell,
      id: 2,
      symbol: "AXISBLUE",
      class: "mutual_fund",
      action: "SELL",
      entry: 45.6721,
      stop: null,
      target: null,
      timing_note: "units at NAV of the 15:00 cutoff; cash arrives T+1/T+2",
    });
    expect(html).toContain("ESTIMATE");
    expect(html).toContain("NAV at today's 15:00 cutoff");
    expect(html).toContain("cash arrives T+2");
    expect(html).not.toContain('data-stamp="sell"');
  });

  test("blocked rows show the shortfall and are hatched, not red", () => {
    const html = renderLedgerRow({
      ...stockSell,
      id: 3,
      action: "BLOCKED",
      reasons: ["short by 12,400"],
      rationale: null,
    });
    expect(html).toContain("insufficient cash");
    expect(html).toContain("short by 12,400");
    expect(html).toContain("data-blocked");
    expect(html).not.toContain('data-stamp="buy"');
  });

  test("headline text is escaped, not injected", () => {
    const html = renderLedgerRow({
      ...stockSell,
      rationale: '<img src=x onerror="alert(1)">',
    });
    expect(html).not.toContain("<img src=x");
    expect(html).toContain("&lt;img");
  });
});

describe("settings", () => {
  const settings = (over: Partial<Settings> = {}): Settings => ({
    budget: 100000,
    committed: 0,
    available: 100000,
    currency: "INR",
    currency_locked: false,
    ...over,
  });

  test("disables save when the budget is below committed", () => {
    const html = renderSettings(settings({ committed: 142000, available: -42000 }));
    expect(html).toContain("Committed is");
    expect(html).toContain("disabled");
  });

  test("disables currency once locked, with the reason", () => {
    const html = renderSettings(settings({ currency_locked: true }));
    expect(html).toContain("Currency is fixed once the ledger has entries");
  });

  test("leaves both controls usable on a fresh ledger", () => {
    const html = renderSettings(settings());
    expect(html).not.toContain("disabled");
  });
});

describe("ledger", () => {
  const dashboard = (signals: SignalRow[]): Dashboard => ({
    currency: "INR",
    budget: 200000,
    committed: 0,
    available: 200000,
    currency_locked: false,
    positions: [],
    signals,
    blocked: [],
  });

  test("empty ledger renders ruled blanks, not a spinner", () => {
    const html = renderLedger(dashboard([]));
    expect(html).toContain("ledger-blank");
    expect(html).toContain("No entries today");
    expect(html).not.toContain("spinner");
  });

  test("balance runs down the page as rows are read", () => {
    const html = renderLedger(
      dashboard([
        { ...stockSell, id: 1, action: "BUY", entry: 100 },
        { ...stockSell, id: 2, action: "BUY", entry: 100 },
      ]),
    );
    expect(html).toContain("1,99,900");
    expect(html).toContain("1,99,800");
  });
});
