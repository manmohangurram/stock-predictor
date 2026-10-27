import type { SignalRow } from "../api";
import { money } from "../format";

export interface Risk {
  /** Money that would be lost if the stop is hit. */
  amount: number;
  /** Loss per unit. */
  perUnit: number;
  /** Reward divided by risk; null when there is no stop or target. */
  rewardRisk: number | null;
  /** Profit at target, as a percentage of entry. */
  profitPct: number | null;
  /** True when the take-profit clears the owner's expectation. */
  meetsTarget: boolean | null;
}

/**
 * Risk is computed from the validated numbers, never reported by the model: a model that can
 * invent a risk figure can invent a comfortable one.
 */
export function riskOf(s: SignalRow, targetProfitPct: number | null): Risk | null {
  if (s.qty === null || s.entry === null || s.stop === null) return null;
  const buy = s.action === "BUY";
  const perUnit = buy ? s.entry - s.stop : s.stop - s.entry;
  if (!(perUnit > 0)) return null;
  const rewardPerUnit =
    s.target === null ? null : buy ? s.target - s.entry : s.entry - s.target;
  const profitPct = s.target === null ? null : ((s.target - s.entry) / s.entry) * 100;
  return {
    amount: perUnit * s.qty,
    perUnit,
    rewardRisk: rewardPerUnit === null ? null : rewardPerUnit / perUnit,
    profitPct,
    meetsTarget:
      targetProfitPct === null || profitPct === null ? null : profitPct + 1e-9 >= targetProfitPct,
  };
}

export function renderRisk(s: SignalRow, targetProfitPct: number | null): string {
  const r = riskOf(s, targetProfitPct);
  if (!r) return "";
  const rr = r.rewardRisk === null ? "—" : `1:${r.rewardRisk.toFixed(1)}`;
  const meets =
    r.meetsTarget === null
      ? ""
      : r.meetsTarget
        ? `<span class="ok-pill" title="Reaches the profit you asked for">meets ${targetProfitPct}%</span>`
        : `<span class="warn-pill" title="Below the profit you asked for">below ${targetProfitPct}%</span>`;
  return `<div class="risk">
    <div><span class="lbl">At risk</span><b class="num down">${money(r.amount, 0)}</b></div>
    <div><span class="lbl">Per unit</span><b class="num">${money(r.perUnit)}</b></div>
    <div><span class="lbl">Reward : risk</span><b class="num">${rr}</b></div>
    ${r.profitPct === null ? "" : `<div><span class="lbl">Profit at target</span><b class="num up">${r.profitPct.toFixed(1)}%</b></div>`}
    ${meets ? `<div class="risk-note">${meets}</div>` : ""}
  </div>`;
}
