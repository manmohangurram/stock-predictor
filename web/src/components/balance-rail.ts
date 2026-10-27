import type { Dashboard, Settings } from "../api";
import { money } from "../format";

export function renderBalanceRail(d: Dashboard | Settings, classFigures?: Record<string, number>): string {
  const negative = d.available < 0;
  const cur = d.currency;
  return `<aside class="rail" aria-label="Balance">
  <span class="label">Available · ${cur}</span>
  <span class="available ${negative ? "negative" : ""}">${money(d.available, 0)}</span>
  <div class="pair"><span>Committed</span><span>${money(d.committed, 0)}</span></div>
  <div class="pair"><span>Budget</span><span>${money(d.budget, 0)}</span></div>
  ${
    classFigures
      ? `<div class="chips">${Object.entries(classFigures)
          .map(([k, v]) => `<div><span>${k}</span><span class="num">${money(v, 0)}</span></div>`)
          .join("")}</div>`
      : ""
  }
  ${
    negative
      ? `<p class="guard">Available is negative: committed exceeds the ceiling. Set a higher budget.</p>`
      : ""
  }
</aside>`;
}
