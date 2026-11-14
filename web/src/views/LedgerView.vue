<script setup lang="ts">
/**
 * The ledger as a view: what is held, what is being asked of it, and the four numbers that decide
 * whether anything is affordable. Data comes from the dashboard endpoint, which already answers 304
 * when nothing changed.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { api, type Dashboard, type SignalRow } from "../api";
import { money } from "../format";
import { railBadges } from "../stores/rail";
import DecisionItem from "../components/DecisionItem.vue";
import HoldingRow from "../components/HoldingRow.vue";
import AddInstrument from "../components/AddInstrument.vue";
import { ITEMS, isDecisionFilter, type DecisionFilter } from "../components/decision-filter";

const route = useRoute();
const router = useRouter();
const data = ref<Dashboard | null>(null);
const error = ref("");
const decisions = ref<DecisionFilter>("pending");

const mode = computed(() => (route.query.mode === "trade" ? "trade" : "long"));
const isTrade = computed(() => mode.value === "trade");

const stocks = computed(() => (data.value?.positions ?? []).filter((p) => p.class === "stock"));
const funds = computed(() => (data.value?.positions ?? []).filter((p) => p.class !== "stock"));
const all = computed(() => {
  const rows = data.value?.positions ?? [];
  return {
    invested: rows.reduce((a, p) => a + p.qty * p.avg_cost, 0),
    value: rows.reduce((a, p) => a + (p.value ?? p.qty * p.avg_cost), 0),
  };
});
const pnl = computed(() => all.value.value - all.value.invested);
const subtotal = (rows: Dashboard["positions"]) => ({
  invested: rows.reduce((a, p) => a + p.qty * p.avg_cost, 0),
  value: rows.reduce((a, p) => a + (p.value ?? p.qty * p.avg_cost), 0),
});

/** A plan is one decision with several legs; the first leg's id is the group's id. */
interface Entry {
  legs: SignalRow[];
  single: boolean;
  fund: boolean;
}
const entries = computed<Entry[]>(() => {
  const shown = [...(data.value?.signals ?? [])].sort((a, b) => b.id - a.id);
  const plans = new Map<number, SignalRow[]>();
  for (const s of shown) {
    if (s.plan_id) plans.set(s.plan_id, [...(plans.get(s.plan_id) ?? []), s]);
  }
  const out: Entry[] = [];
  const seen = new Set<number>();
  for (const s of shown.slice(0, 10)) {
    if (s.plan_id && plans.has(s.plan_id)) {
      if (seen.has(s.plan_id)) continue;
      seen.add(s.plan_id);
      // Sells first, as they must be executed.
      const legs = [...plans.get(s.plan_id)!].sort(
        (a, b) => (a.action === "SELL" ? -1 : 1) - (b.action === "SELL" ? -1 : 1),
      );
      out.push({ legs, single: false, fund: legs[0].plan_scope === "fund" });
      continue;
    }
    out.push({ legs: [s], single: true, fund: s.class === "mutual_fund" });
  }
  return out;
});

const signalFor = (symbol: string): SignalRow | undefined =>
  (data.value?.signals ?? []).find((s) => s.symbol === symbol);

async function load(): Promise<void> {
  try {
    data.value = await api.dashboard(mode.value);
    railBadges.ledger = String(data.value.positions.length);
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

function setMode(next: "long" | "trade"): void {
  void router.push({ path: "/", query: { mode: next } });
}

const fundsOnly = computed(() => entries.value.filter((e) => e.fund));
const stocksOnly = computed(() => entries.value.filter((e) => !e.fund));
const both = computed(() => fundsOnly.value.length > 0 && stocksOnly.value.length > 0);

let timer: number | undefined;
onMounted(async () => {
  await load();
  timer = window.setInterval(() => void load(), 60_000);
});
onBeforeUnmount(() => window.clearInterval(timer));
watch(mode, load);
</script>

<template>
  <div v-if="error" class="error-band">Something failed: {{ error }}
    <button class="d-btn d-btn-sm" @click="load()">Retry</button>
  </div>
  <template v-else-if="data">
    <div class="page-head">
      <div>
        <h1>{{ isTrade ? "Short-term book" : "Holdings" }}</h1>
        <div class="page-sub">
          {{
            isTrade
              ? "Short-term pot · separate capital, separate limits"
              : `Long-term book · ${stocks.length} ${stocks.length === 1 ? "stock" : "stocks"} and ${funds.length} ${funds.length === 1 ? "fund" : "funds"}`
          }}
        </div>
      </div>
      <div class="head-right">
        <div class="head-row">
          <div class="figures">
            <div><span class="lbl">Invested</span>
              <span class="fig-line"><b class="num">{{ money(all.invested, 0) }}</b></span></div>
            <div><span class="lbl">Now</span>
              <span class="fig-line"><b class="num">{{ money(all.value, 0) }}</b></span></div>
            <div><span class="lbl">Unrealised</span>
              <span class="fig-line"><b class="num" :class="pnl === 0 ? '' : pnl > 0 ? 'pos' : 'neg'">{{
                pnl === 0 ? money(0, 0) : `${pnl > 0 ? "+" : "−"}${money(Math.abs(pnl), 0)}`
              }}</b></span></div>
            <div><span class="lbl">Free to deploy</span>
              <span class="fig-line"><b class="num">{{ money(data.available, 0) }}</b>
                <span v-if="data.budget > 0" class="fig-note">of {{ money(data.budget, 0) }}</span></span></div>
          </div>
          <AddInstrument />
        </div>
      </div>
    </div>

    <div class="mode-row">
      <nav class="modes" aria-label="Strategy">
        <a class="mode-tab" href="/?mode=long" :aria-current="!isTrade ? 'page' : undefined" @click.prevent="setMode('long')">Long-term</a>
        <a class="mode-tab" href="/?mode=trade" :aria-current="isTrade ? 'page' : undefined" @click.prevent="setMode('trade')">Short-term</a>
      </nav>
      <span class="sub">{{ data.currency }} · as of the last poll</span>
    </div>

    <section class="card" id="decisions-card">
      <div class="card-head">
        <div class="lbl">Decisions</div>
        <div class="seg-filter" role="group" aria-label="Decision status">
          <a
            v-for="it in ITEMS"
            :key="it.id"
            :href="`/?mode=${mode}&decisions=${it.id}`"
            class="seg-item"
            :class="{ on: decisions === it.id }"
            :aria-current="decisions === it.id ? 'page' : undefined"
            :title="`${it.label} — ${it.hint}`"
            :aria-label="it.label"
            @click.prevent="decisions = isDecisionFilter(it.id) ? it.id : 'pending'"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
                 stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"
                 aria-hidden="true" v-html="it.icon" />
            <span>{{ it.label }}</span>
          </a>
        </div>
      </div>
      <div class="feed decisions-feed">
        <template v-if="entries.length">
          <template v-if="stocksOnly.length">
            <div v-if="both" class="decision-group"><span class="lbl">Stocks</span></div>
            <DecisionItem
              v-for="e in stocksOnly"
              :key="e.legs[0].id"
              :legs="e.legs"
              :single="e.single"
              @reload="load()"
            />
          </template>
          <template v-if="fundsOnly.length">
            <div class="decision-group"><span class="lbl">Funds</span></div>
            <DecisionItem
              v-for="e in fundsOnly"
              :key="e.legs[0].id"
              :legs="e.legs"
              :single="e.single"
              @reload="load()"
            />
          </template>
        </template>
        <p v-else class="empty">{{ decisions === "pending" ? "Nothing under review." : "Nothing in this filter." }}</p>
      </div>
    </section>

    <!-- The trade book is one table; the long book is stocks and funds. -->
    <section v-if="isTrade" class="sect">
      <div class="sect-head"><h2>Short-term positions</h2><span class="sub">{{ data.positions.length }} · all stocks</span></div>
      <div class="table-wrap"><table class="blotter">
        <thead><tr>
          <th scope="col">Holding</th><th scope="col" class="hide-sm num">Qty</th>
          <th scope="col" class="hide-sm num">Bought</th><th scope="col" class="hide-sm num">Invested</th>
          <th scope="col" class="num">Now</th><th scope="col" class="hide-sm num">Value</th>
          <th scope="col" class="num">P&amp;L</th><th scope="col">Signal</th>
          <th scope="col" class="hide-sm num">Exit plan</th>
        </tr></thead>
        <tbody>
          <HoldingRow v-for="p in data.positions" :key="p.id" :position="p" :signal="signalFor(p.symbol)" :mode="mode" />
          <tr v-if="!data.positions.length"><td colspan="9" class="empty">Nothing in the short-term pot.</td></tr>
        </tbody>
      </table></div>
    </section>

    <template v-else>
      <section class="sect">
        <div class="sect-head"><h2>Stocks</h2><span class="sub">{{ stocks.length }} · exit plan from the last signal</span></div>
        <div class="table-wrap"><table class="blotter">
          <thead><tr>
            <th scope="col">Holding</th><th scope="col" class="hide-sm num">Qty</th>
            <th scope="col" class="hide-sm num">Bought</th><th scope="col" class="hide-sm num">Invested</th>
            <th scope="col" class="num">Now</th><th scope="col" class="hide-sm num">Value</th>
            <th scope="col" class="num">P&amp;L</th><th scope="col">Signal</th>
            <th scope="col" class="hide-sm num">Exit plan</th>
          </tr></thead>
          <tbody>
            <HoldingRow v-for="p in stocks" :key="p.id" :position="p" :signal="signalFor(p.symbol)" :mode="mode" />
            <tr v-if="!stocks.length"><td colspan="9" class="empty">No stocks yet — add one from the header.</td></tr>
            <tr v-if="stocks.length" class="subtotal">
              <td>Stocks</td><td class="hide-sm">—</td><td class="hide-sm">—</td>
              <td class="num hide-sm">{{ money(subtotal(stocks).invested) }}</td><td>—</td>
              <td class="num hide-sm">{{ money(subtotal(stocks).value) }}</td>
              <td class="num" :class="subtotal(stocks).value >= subtotal(stocks).invested ? 'up' : 'down'">{{
                `${subtotal(stocks).value >= subtotal(stocks).invested ? "+" : "−"}${money(Math.abs(subtotal(stocks).value - subtotal(stocks).invested))}`
              }}</td>
              <td>—</td><td class="sub hide-sm">—</td>
            </tr>
          </tbody>
        </table></div>
      </section>

      <section class="sect">
        <div class="sect-head"><h2>Mutual funds</h2><span class="sub">{{ funds.length }} · bought at NAV · exit at the NAV target</span></div>
        <div class="table-wrap"><table class="blotter">
          <thead><tr>
            <th scope="col">Scheme</th><th scope="col" class="hide-sm num">Units</th>
            <th scope="col" class="hide-sm num">Avg NAV</th><th scope="col" class="hide-sm num">Invested</th>
            <th scope="col" class="num">NAV today</th><th scope="col" class="hide-sm num">Value</th>
            <th scope="col" class="num">P&amp;L</th><th scope="col">Signal</th>
            <th scope="col" class="hide-sm num">Exit plan</th>
          </tr></thead>
          <tbody>
            <HoldingRow v-for="p in funds" :key="p.id" :position="p" :signal="signalFor(p.symbol)" :mode="mode" />
            <tr v-if="!funds.length"><td colspan="9" class="empty">No funds yet. A fund is added at NAV and held.</td></tr>
            <tr v-if="funds.length" class="subtotal">
              <td>Funds</td><td class="hide-sm">—</td><td class="hide-sm">—</td>
              <td class="num hide-sm">{{ money(subtotal(funds).invested) }}</td><td>—</td>
              <td class="num hide-sm">{{ money(subtotal(funds).value) }}</td>
              <td class="num" :class="subtotal(funds).value >= subtotal(funds).invested ? 'up' : 'down'">{{
                `${subtotal(funds).value >= subtotal(funds).invested ? "+" : "−"}${money(Math.abs(subtotal(funds).value - subtotal(funds).invested))}`
              }}</td>
              <td>—</td><td class="sub hide-sm">—</td>
            </tr>
          </tbody>
        </table></div>
      </section>
    </template>
  </template>
</template>
