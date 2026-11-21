import type { SignalRow } from "../api";

/** What one leg costs: a fund move is sized in rupees, a stock leg in units at its entry. */
export const legValue = (l: SignalRow): number =>
  l.amount ?? (l.qty ?? 0) * (l.entry ?? 0);

const isBuy = (l: SignalRow): boolean => l.action !== "SELL";

/** The net debit a plan asks of the purse: buys less the money its sells free. */
export const debitOf = (legs: SignalRow[]): number => {
  const out = legs.filter((l) => !isBuy(l)).reduce((n, l) => n + legValue(l), 0);
  const into = legs.filter(isBuy).reduce((n, l) => n + legValue(l), 0);
  return Math.max(0, into - out);
};

/** Reward over risk, in the units a stop defines. Null when any of the three prices is missing. */
export const rewardRisk = (l: SignalRow): number | null => {
  if (l.entry === null || l.stop === null || l.target === null) return null;
  const risk = l.entry - l.stop;
  if (risk <= 0) return null;
  return (l.target - l.entry) / risk;
};
