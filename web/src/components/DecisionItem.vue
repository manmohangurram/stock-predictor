<script setup lang="ts">
/**
 * One decision, and the two things you can do about it.
 *
 * A plan is one decision with several legs: read as a whole, approved as a whole, with an editable
 * fill per leg — you rarely get the exact price a proposal named, and half a swap is not a swap.
 */
import { computed, ref } from "vue";
import type { SignalRow } from "../api";
import { api } from "../api";
import { day, histDate, money, units } from "../format";
import { actionStamp, stamp, statusPill, toneOf } from "./stamp";
import { renderConfidence } from "./confidence";

const props = defineProps<{ legs: SignalRow[]; single: boolean }>();
const emit = defineEmits<{ reload: [] }>();

const first = computed(() => props.legs[0]);
const pending = computed(() => first.value.status === "pending");
const isFundPlan = computed(() => first.value.plan_scope === "fund");

const form = ref<"none" | "approve" | "reject">("none");
const status = ref("");
const reason = ref("");
/** One editable fill per leg, keyed by signal id: qty (or amount for a fund) and price. */
const fills = ref<Record<number, { qty: string; price: string }>>({});

function open(kind: "approve" | "reject"): void {
  form.value = form.value === kind ? "none" : kind;
  status.value = "";
  if (kind === "approve") {
    fills.value = Object.fromEntries(
      props.legs.map((l) => [
        l.id,
        {
          qty: String(isFundPlan.value ? (l.amount ?? "") : (l.qty ?? "")),
          price: String(l.entry ?? ""),
        },
      ]),
    );
  }
}

/** Units round down to 3 decimals, the same floor the ledger applies when it costs a buy. */
const floorUnits = (n: number): number => Math.floor(n * 1000) / 1000;

async function post(): Promise<void> {
  status.value = "Posting…";
  try {
    if (props.single) {
      const leg = first.value;
      const f = fills.value[leg.id] ?? { qty: "", price: "" };
      const price = Number(f.price);
      const amount = Number(f.qty);
      const qty =
        isFundPlan.value && amount > 0 && price > 0 ? floorUnits(amount / price) : Number(f.qty);
      await api.approveSignal(leg.id, { qty, price, at: undefined });
    } else {
      await api.approveSignal(first.value.id, {
        fills: props.legs.map((l) => {
          const f = fills.value[l.id] ?? { qty: "", price: "" };
          const price = Number(f.price);
          const amount = Number(f.qty);
          const units =
            isFundPlan.value && amount > 0 && price > 0 ? floorUnits(amount / price) : Number(f.qty);
          return { id: l.id, qty: units, price };
        }),
      });
    }
    form.value = "none";
    emit("reload");
  } catch (e) {
    status.value = String(e);
  }
}

async function reject(): Promise<void> {
  status.value = "Rejecting…";
  try {
    await api.rejectSignal(first.value.id, reason.value || undefined);
    form.value = "none";
    emit("reload");
  } catch (e) {
    status.value = String(e);
  }
}

const value = (l: SignalRow): number => l.amount ?? (l.qty ?? 0) * (l.entry ?? 0);
const freed = computed(() =>
  props.legs.filter((l) => l.action === "SELL").reduce((n, l) => n + value(l), 0),
);
const spent = computed(() =>
  props.legs.filter((l) => l.action === "BUY").reduce((n, l) => n + value(l), 0),
);
const prose = computed(() => {
  const s = first.value;
  return props.single ? (s.rationale ?? s.reasons[0] ?? "") : (s.plan_rationale ?? "");
});
</script>

<template>
  <div class="feed-row decision-feed" :class="{ 'plan-card': !single }">
    <div class="decision-main">
      <div v-if="!single">
        <span v-html="stamp('estimate', isFundPlan ? 'FUND PLAN' : 'STOCK PLAN')" />
        <b>{{ legs.length }} move{{ legs.length === 1 ? "" : "s" }}</b>
        <span v-if="first.status !== 'pending'" v-html="statusPill(first.status)" />
        <span class="sub">{{ histDate(first.created_at) }}</span>
      </div>

      <!-- A plan: one docket line per leg, then the purse arithmetic. -->
      <div v-if="!single" class="docket">
        <div v-for="l in legs" :key="l.id" class="docket-move">
          <span v-html="actionStamp(l.action)" />
          <span class="docket-sym">{{ l.name || l.symbol }}</span>
          <span class="docket-figs">
            <template v-if="l.qty !== null && l.qty > 0">
              <span class="docket-fig"><span class="lbl">Qty</span><b class="num">{{ units(l.qty!) }}</b></span>
              <span v-if="l.entry !== null" class="docket-fig"><span class="lbl">At</span><b class="num">{{ money(l.entry) }}</b></span>
              <span v-if="l.action === 'SELL' && l.entry !== null" class="docket-fig"><span class="lbl">Gives</span><b class="num">{{ money(value(l)) }}</b></span>
              <span v-if="l.action === 'BUY' && l.stop !== null" class="docket-fig"><span class="lbl">Stop</span><b class="num">{{ money(l.stop) }}</b></span>
              <span v-if="l.action === 'BUY' && l.target !== null" class="docket-fig"><span class="lbl">Target</span><b class="num">{{ money(l.target) }}</b></span>
              <span v-if="l.action === 'BUY' && l.valid_until" class="docket-fig"><span class="lbl">Till</span><b class="num">{{ day(l.valid_until) }}</b></span>
            </template>
            <span v-else-if="l.amount !== null" class="docket-fig"><span class="lbl">Amount</span><b class="num">{{ money(l.amount) }}</b></span>
            <span v-else class="lbl">size on approval</span>
          </span>
        </div>
        <div v-if="freed > 0 && spent > 0" class="docket-purse">
          <div class="docket-row"><span class="lbl">From the sale</span><span class="num">{{ money(freed) }}</span></div>
          <div class="docket-row"><span class="lbl">To buy</span><span class="num">{{ money(spent) }}</span></div>
          <div v-if="Math.abs(spent - freed) >= 0.005" class="docket-row total">
            <span class="lbl">{{ spent > freed ? "Also from the purse" : "Back to the purse" }}</span>
            <span class="num">{{ money(Math.abs(spent - freed)) }}</span>
          </div>
        </div>
        <div v-else-if="spent > 0" class="docket-purse">
          <div class="docket-row total"><span class="lbl">Cost</span><span class="num">{{ money(spent) }}</span></div>
        </div>
        <div v-else-if="freed > 0" class="docket-purse">
          <div class="docket-row total"><span class="lbl">Frees</span><span class="num">{{ money(freed) }}</span></div>
        </div>
      </div>

      <!-- A single decision. -->
      <div v-if="single">
        <span v-html="actionStamp(first.action)" />
        <b :title="first.symbol">{{ first.name && first.name !== first.symbol ? first.name : first.symbol }}</b>
        <span v-if="first.name && first.name !== first.symbol" class="sub">{{ first.symbol }}</span>
        <span v-if="first.entry !== null" class="num">{{ money(first.entry) }}</span>
        <span v-if="first.qty === null || first.qty === 0" class="lbl">size on approval</span>
        <span v-else class="lbl">{{ units(first.qty) }} units</span>
        <span v-if="first.status !== 'pending'" v-html="statusPill(first.status)" />
        <span class="sub">{{ histDate(first.created_at) }}</span>
        <span class="signal-cell" v-html="renderConfidence(first.confidence)" />
      </div>

      <p v-if="prose" class="decision-prose">{{ prose }}</p>

      <div v-if="pending" class="decision-actions">
        <button
          class="d-btn d-btn-sm d-btn-ghost d-btn-square"
          type="button"
          :aria-expanded="form === 'approve'"
          :title="single ? 'Approve — record the fill you actually got' : 'Approve the plan — record the fill you got on each leg'"
          aria-label="Approve"
          @click="open('approve')"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.4 8.4l3 3 7-7" /></svg>
        </button>
        <button
          class="d-btn d-btn-sm d-btn-ghost d-btn-square"
          type="button"
          :aria-expanded="form === 'reject'"
          title="Reject — with a reason, no money moves"
          aria-label="Reject"
          @click="open('reject')"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
        </button>
      </div>

      <form v-if="form === 'approve'" class="approve-form" @submit.prevent="post">
        <div v-if="!single" class="plan-fills">
          <div class="plan-fill plan-fill-head" aria-hidden="true">
            <span></span><span>Holding</span>
            <span>{{ isFundPlan ? "Amount ₹" : "Quantity" }}</span>
            <span>{{ isFundPlan ? "NAV" : "Price" }}</span>
          </div>
          <div v-for="l in legs" :key="l.id" class="plan-fill">
            <span v-html="actionStamp(l.action)" />
            <span class="plan-fill-name">{{ l.name || l.symbol }}<span class="sub">{{ l.symbol }}</span></span>
            <input
              v-model="fills[l.id].qty"
              type="number"
              min="0"
              step="any"
              :aria-label="`${isFundPlan ? 'Amount' : 'Quantity'} for ${l.name || l.symbol}`"
            />
            <input
              v-model="fills[l.id].price"
              type="number"
              min="0"
              step="any"
              :aria-label="`${isFundPlan ? 'NAV' : 'Price'} for ${l.name || l.symbol}`"
            />
          </div>
        </div>
        <div v-else class="fill-grid">
          <label class="field"><span class="lbl">{{ isFundPlan ? "Amount" : "Quantity" }}</span>
            <input v-model="fills[first.id].qty" type="number" min="0" step="any" /></label>
          <label class="field"><span class="lbl">{{ isFundPlan ? "NAV" : "Price" }}</span>
            <input v-model="fills[first.id].price" type="number" min="0" step="any" /></label>
        </div>
        <div class="row">
          <button class="d-btn d-btn-sm d-btn-primary" type="submit">{{ single ? "Post fill" : "Post fills" }}</button>
          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="form = 'none'">Cancel</button>
          <span class="help" role="status">{{ status }}</span>
        </div>
      </form>

      <form v-if="form === 'reject'" class="reject-form" @submit.prevent="reject">
        <div class="row">
          <input v-model="reason" placeholder="Why not? (optional)" aria-label="Reason" />
          <button class="d-btn d-btn-sm" type="submit">Reject decision</button>
          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="form = 'none'">Cancel</button>
          <span class="help" role="status">{{ status }}</span>
        </div>
      </form>
    </div>
  </div>
</template>
