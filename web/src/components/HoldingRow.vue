<script setup lang="ts">
/** One holdings row. Name first: a ticker tells you nothing about what you own. */
import { computed } from "vue";
import type { Dashboard, PositionRow, SignalRow } from "../api";
import { money, units } from "../format";
import { renderConfidence } from "./confidence";
import { stamp, toneOf } from "./stamp";

const props = defineProps<{
  position: PositionRow;
  signal?: SignalRow;
  mode: Dashboard["mode"];
}>();

const p = computed(() => props.position);
const live = computed(() => p.value.ltp !== null);
const ltp = computed(() => p.value.ltp ?? p.value.avg_cost);
const invested = computed(() => p.value.qty * p.value.avg_cost);
const value = computed(() => p.value.value ?? invested.value);
const pnl = computed(() => value.value - invested.value);
const pnlPct = computed(() => (invested.value > 0 ? (pnl.value / invested.value) * 100 : 0));
const sign = (n: number): string => (n === 0 ? "" : n > 0 ? "+" : "−");
const isFund = computed(() => p.value.class !== "stock");

const exitPlan = computed(() => {
  if (isFund.value) {
    return p.value.target_profit_pct
      ? `${money(p.value.avg_cost * (1 + p.value.target_profit_pct / 100))} → +${p.value.target_profit_pct}%`
      : "no target set";
  }
  if (p.value.stop === null && p.value.target === null) return "no exit set";
  return `${p.value.stop === null ? "—" : money(p.value.stop)} → ${p.value.target === null ? "—" : money(p.value.target)}`;
});
</script>

<template>
  <tr class="row" :data-symbol="p.symbol" tabindex="0" role="link" :aria-label="`${p.name}, open details`">
    <td class="holding" :title="`${p.name} · ${p.symbol}`">
      <div class="name">{{ p.name }}</div>
      <div class="sub">{{ p.symbol }}</div>
    </td>
    <td class="num hide-sm">{{ units(p.qty) }}</td>
    <td class="num hide-sm">{{ money(p.avg_cost) }}</td>
    <td class="num hide-sm">{{ money(invested) }}</td>
    <td class="num">
      <span v-if="live">{{ money(ltp) }}</span><span v-else class="muted">—</span>
    </td>
    <td class="num hide-sm">
      <span v-if="live">{{ money(value) }}</span><span v-else class="muted">—</span>
    </td>
    <td class="num" :class="live ? (pnl >= 0 ? 'up' : 'down') : ''">
      <template v-if="live">
        {{ sign(pnl) }}{{ money(Math.abs(pnl)) }}
        <div class="sub">{{ sign(pnlPct) }}{{ Math.abs(pnlPct).toFixed(1) }}%</div>
      </template>
      <span v-else class="muted">no price yet</span>
    </td>
    <td class="signal-col">
      <span v-if="signal" class="signal-cell">
        <span v-html="stamp(toneOf(signal.action), signal.action)" />
        <span v-html="renderConfidence(signal.confidence)" />
      </span>
      <span v-else class="muted">—</span>
    </td>
    <td class="right hide-sm num-text">
      <span v-if="exitPlan.startsWith('no ')" class="muted">{{ exitPlan }}</span>
      <span v-else class="num">{{ exitPlan }}</span>
    </td>
  </tr>
</template>
