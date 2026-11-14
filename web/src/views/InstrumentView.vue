<script setup lang="ts">
/**
 * The instrument: the chart and its evidence, with the numbers you act on beside it.
 *
 * Everything here is about one holding, so the page is a two-column reading order — the tape and
 * what you did, then the exit, the expectation, the rules and the two dangerous buttons.
 */
import { computed, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { api, type Analysis, type InstrumentDetail, type SignalRow, type Trade } from "../api";
import { clock, day, money, units } from "../format";
import { pill, statusPill } from "../components/stamp";
import { ITEMS, isDecisionFilter, type DecisionFilter } from "../components/decision-filter";
import DecisionItem from "../components/DecisionItem.vue";
import PriceChart from "../components/PriceChart.vue";

const route = useRoute();
const analysis = ref<Analysis | null>(null);
const detail = ref<InstrumentDetail | null>(null);
const trades = ref<Trade[]>([]);
const error = ref("");
const decisions = ref<DecisionFilter>("all");

type Ledger = Analysis & { trades: Trade[] };

const symbol = computed(() => String(route.params.symbol ?? ""));

async function load(): Promise<void> {
  try {
    const a = (await api.analysis(symbol.value)) as Ledger;
    analysis.value = a;
    detail.value = a as unknown as InstrumentDetail;
    trades.value = a.trades ?? [];
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(load);
watch(symbol, load);

const a = computed(() => analysis.value);
const inst = computed(() => a.value?.instrument);
const position = computed(() => a.value?.position ?? null);
const klass = computed(() => inst.value?.class ?? "stock");
const isFund = computed(() => klass.value === "mutual_fund");
const archived = computed(() => !inst.value?.active);

const ltp = computed(() =>
  a.value?.bars.length ? a.value.bars[a.value.bars.length - 1].close : (position.value?.avg_cost ?? null),
);
const invested = computed(() =>
  position.value ? position.value.qty * position.value.avg_cost : null,
);
const value = computed(() =>
  position.value && ltp.value !== null ? position.value.qty * ltp.value : null,
);
const unrealised = computed(() =>
  invested.value !== null && value.value !== null ? value.value - invested.value : null,
);
const realised = computed(() => trades.value.reduce((n, t) => n + (t.realised ?? 0), 0));
const closedOut = computed(() => trades.value.length > 0 && realised.value !== 0);
const sign = (n: number): string => (n >= 0 ? "+" : "−");
const signed = (n: number): string => `${sign(n)}${money(Math.abs(n))}`;

const facts = computed(() => {
  const ind = a.value?.indicators;
  if (!ind) return [];
  return [
    ["SMA50", money(ind.sma50)],
    ["SMA200", money(ind.sma200)],
    ["RSI14", ind.rsi14.toFixed(1)],
    ["ATR14", money(ind.atr14)],
    ["52w low", money(ind.low52)],
    ["52w high", money(ind.high52)],
  ];
});

const shownSignals = computed<SignalRow[]>(() => {
  const all = detail.value?.signals ?? [];
  return decisions.value === "all" ? all : all.filter((s) => s.status === decisions.value);
});

/* ---- recording a fill ---------------------------------------------------- */
const recording = ref(false);
const side = ref<"buy" | "sell">("buy");
const fill = ref({ qty: "", price: "", at: new Date().toISOString().slice(0, 10), mode: "long" });
const tradeStatus = ref("");

async function record(): Promise<void> {
  tradeStatus.value = "Recording…";
  try {
    await api.recordTrade(symbol.value, {
      side: side.value,
      qty: Number(fill.value.qty),
      price: Number(fill.value.price),
      at: fill.value.at,
      mode: fill.value.mode,
    } as never);
    recording.value = false;
    fill.value = { qty: "", price: "", at: new Date().toISOString().slice(0, 10), mode: "long" };
    tradeStatus.value = "";
    await load();
  } catch (e) {
    tradeStatus.value = String(e);
  }
}

/* ---- correcting and deleting a fill -------------------------------------- */
const editing = ref<number | null>(null);
const edit = ref({ qty: "", price: "", at: "" });
const fillStatus = ref("");
const armed = ref<number | null>(null);

function openEdit(t: Trade): void {
  editing.value = t.id;
  edit.value = { qty: String(t.qty), price: String(t.price), at: t.traded_at.slice(0, 10) };
  fillStatus.value = "";
}

async function saveEdit(id: number): Promise<void> {
  fillStatus.value = "Saving…";
  try {
    await api.editTrade(id, {
      qty: Number(edit.value.qty),
      price: Number(edit.value.price),
      at: edit.value.at,
    } as never);
    editing.value = null;
    await load();
  } catch (e) {
    fillStatus.value = String(e);
  }
}

async function deleteFill(id: number): Promise<void> {
  if (armed.value !== id) {
    armed.value = id;
    window.setTimeout(() => {
      if (armed.value === id) armed.value = null;
    }, 4000);
    return;
  }
  try {
    await api.deleteTrade(id);
    armed.value = null;
    editing.value = null;
    await load();
  } catch (e) {
    fillStatus.value = String(e);
  }
}

const vsClose = (t: Trade): { text: string; tone: string } => {
  if (t.market_close === null) return { text: "—", tone: "" };
  const diff = (t.price - t.market_close) / t.market_close;
  return {
    text: `${diff >= 0 ? "+" : ""}${(diff * 100).toFixed(2)}%`,
    tone: diff > 0.0005 ? "up" : diff < -0.0005 ? "down" : "",
  };
};

/* ---- expected profit, refresh, archive ----------------------------------- */
const target = ref("");
const refreshStatus = ref("Fetches this symbol now, instead of waiting for the next poll.");
const manageStatus = ref("");
const archiving = ref(false);

watch(inst, (i) => {
  target.value = i?.target_profit_pct == null ? "" : String(i.target_profit_pct);
});

async function saveTarget(): Promise<void> {
  const raw = target.value.trim();
  try {
    await api.setTargetProfit(symbol.value, raw === "" ? null : Number(raw));
    await load();
  } catch (e) {
    manageStatus.value = String(e);
  }
}

async function refreshNow(): Promise<void> {
  refreshStatus.value = "Fetching…";
  try {
    const r = await api.refreshInstrument(symbol.value);
    refreshStatus.value = `Stored ${r.bars} bars and ${r.headlines} headlines.`;
    await load();
  } catch (e) {
    refreshStatus.value = String(e);
  }
}

async function archive(): Promise<void> {
  if (!archiving.value) {
    archiving.value = true;
    manageStatus.value = "Archiving clears prices and headlines; fills and decisions stay.";
    window.setTimeout(() => (archiving.value = false), 5000);
    return;
  }
  try {
    await api.archiveInstrument(symbol.value);
    await load();
  } catch (e) {
    manageStatus.value = String(e);
  }
}

async function restore(): Promise<void> {
  try {
    await api.restoreInstrument(symbol.value);
    await load();
  } catch (e) {
    manageStatus.value = String(e);
  }
}
</script>

<template>
  <div v-if="error" class="error-band">Something failed: {{ error }}
    <button class="d-btn d-btn-sm" @click="load()">Retry</button>
  </div>
  <template v-else-if="a && inst">
    <header class="page-head">
      <div class="who">
        <a class="back" href="/">← Ledger</a>
        <h1 :title="inst.name">{{ inst.name }}</h1>
        <div class="page-sub">{{ inst.symbol }} · {{ inst.class.replace("_", " ") }}</div>
      </div>
      <div class="figures">
        <div><span class="lbl">Last</span><b>{{ ltp === null ? "—" : money(ltp) }}</b></div>
        <div><span class="lbl">Held</span><b>{{ position ? `${units(position.qty)} @ ${money(position.avg_cost)}` : "—" }}</b></div>
        <div><span class="lbl">Value</span><b>{{ value === null ? "—" : money(value) }}</b></div>
        <div v-if="archived">
          <span class="lbl">Realised</span>
          <b :class="realised >= 0 ? 'up' : 'down'">{{ closedOut ? signed(realised) : "—" }}</b>
          <div class="sub">from {{ trades.length }} fill{{ trades.length === 1 ? "" : "s" }}</div>
        </div>
        <div v-else>
          <span class="lbl">Unrealised</span>
          <b :class="unrealised === null ? '' : unrealised >= 0 ? 'up' : 'down'">{{ unrealised === null ? "—" : signed(unrealised) }}</b>
        </div>
      </div>
    </header>

    <div class="inst-grid">
      <div class="inst-main">
        <section class="card chart-card">
          <div class="card-head">
            <div class="head-title">
              <h2>{{ isFund ? "NAV" : "Price" }}</h2>
              <span class="sub">{{ a.bars.length }} {{ isFund ? "NAV points" : "sessions" }} ·
                {{ isFund ? "units are allotted at NAV" : "daily close" }}</span>
            </div>
          </div>
          <PriceChart :analysis="a" />
        </section>

        <section class="card">
          <div class="card-head">
            <div class="head-title">
              <h2>Fills</h2>
              <span class="sub">{{ trades.length ? `${trades.length} recorded · what you actually paid` : "nothing recorded yet" }}</span>
            </div>
            <button class="d-btn d-btn-sm" type="button" :aria-expanded="recording" @click="recording = !recording">Record fill</button>
          </div>

          <form v-if="recording" class="trade-form" @submit.prevent="record">
            <div class="seg" role="radiogroup" aria-label="Buy or sell">
              <label class="seg-option" :class="{ on: side === 'buy' }">
                <input v-model="side" type="radio" name="side" value="buy" /><span>Buy</span>
              </label>
              <label class="seg-option" :class="{ on: side === 'sell' }">
                <input v-model="side" type="radio" name="side" value="sell" /><span>Sell</span>
              </label>
            </div>
            <div class="fill-grid">
              <label class="field"><span class="lbl">{{ isFund ? "Units" : "Quantity" }}</span>
                <input v-model="fill.qty" name="qty" type="number" min="0" step="any" required
                       :placeholder="position ? `held ${units(position.qty)}` : (isFund ? 'Units' : 'Qty')" /></label>
              <label class="field"><span class="lbl">{{ isFund ? "NAV" : "Price" }}</span>
                <input v-model="fill.price" name="price" type="number" min="0" step="any" required
                       :placeholder="isFund ? 'NAV' : 'Price'" /></label>
              <label class="field"><span class="lbl">Date</span>
                <input v-model="fill.at" name="at" type="date" :max="new Date().toISOString().slice(0, 10)" /></label>
              <label class="field"><span class="lbl">Pot</span>
                <select v-if="klass === 'stock'" v-model="fill.mode" name="mode">
                  <option value="long">Long-term</option><option value="trade">Short-term</option>
                </select>
                <span v-else class="static">Long-term only</span></label>
            </div>
            <button class="d-btn d-btn-sm d-btn-primary wide" type="submit">Record fill</button>
            <p class="help" role="status">{{ tradeStatus }} A buy is limited by cash; a sell by what you hold.</p>
          </form>

          <div v-if="trades.length" class="table-wrap">
            <table class="blotter fills">
              <thead><tr>
                <th scope="col">Date</th><th scope="col">Side</th><th scope="col" class="num">Qty</th>
                <th scope="col" class="num">Your price</th><th scope="col" class="hide-sm num">Close that day</th>
                <th scope="col" class="hide-sm num">vs close</th><th scope="col"></th>
              </tr></thead>
              <tbody>
                <template v-for="t in trades" :key="t.id">
                  <tr>
                    <td class="mono">{{ day(t.traded_at) }}</td>
                    <td><span class="stamp" :data-stamp="t.side === 'buy' ? 'buy' : 'sell'">{{ t.side.toUpperCase() }}</span></td>
                    <td class="num">{{ units(t.qty) }}</td>
                    <td class="num">{{ money(t.price) }}</td>
                    <td class="num hide-sm">{{ t.market_close === null ? "—" : money(t.market_close) }}</td>
                    <td class="num hide-sm" :class="vsClose(t).tone">{{ vsClose(t).text }}</td>
                    <td class="right">
                      <button class="d-btn d-btn-xs d-btn-ghost d-btn-square" type="button"
                              :aria-expanded="editing === t.id" title="Correct this fill" aria-label="Correct this fill"
                              @click="openEdit(t)">
                        <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor"
                             stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                          <path d="M11.2 2.3l2.5 2.5-8 8-3.2.7.7-3.2 8-8z" /></svg>
                      </button>
                    </td>
                  </tr>
                  <tr v-if="editing === t.id">
                    <td colspan="7">
                      <form class="edit-fill-form" @submit.prevent="saveEdit(t.id)">
                        <div class="fill-grid">
                          <label class="field"><span class="lbl">Quantity</span>
                            <input v-model="edit.qty" name="qty" type="number" min="0" step="any" /></label>
                          <label class="field"><span class="lbl">Price</span>
                            <input v-model="edit.price" name="price" type="number" min="0" step="any" /></label>
                          <label class="field"><span class="lbl">Date</span>
                            <input v-model="edit.at" name="at" type="date" :max="new Date().toISOString().slice(0, 10)" /></label>
                        </div>
                        <div class="row">
                          <button class="d-btn d-btn-sm d-btn-primary" type="submit">Save</button>
                          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="editing = null">Cancel</button>
                          <button class="d-btn d-btn-sm d-btn-ghost danger push-right" type="button"
                                  :data-armed="armed === t.id" @click="deleteFill(t.id)">{{
                            armed === t.id ? "Delete? Click again" : "Delete fill"
                          }}</button>
                          <span class="help" role="status">{{ fillStatus }}</span>
                        </div>
                      </form>
                    </td>
                  </tr>
                </template>
              </tbody>
            </table>
          </div>
          <p v-else class="empty">No fills recorded for this instrument.</p>
        </section>

        <section class="card">
          <div class="card-head">
            <div class="head-title">
              <h2>Decisions</h2>
              <span class="sub">{{ (detail?.signals ?? []).length }}{{ (detail?.signals ?? []).length ? "" : " — nothing yet" }}</span>
            </div>
            <div class="seg-filter" role="group" aria-label="Decision status">
                <a v-for="it in ITEMS" :key="it.id" :href="`?decisions=${it.id}`" class="seg-item"
                   :class="{ on: decisions === it.id }" :aria-current="decisions === it.id ? 'page' : undefined"
                   :title="`${it.label} — ${it.hint}`" :aria-label="it.label"
                   @click.prevent="decisions = isDecisionFilter(it.id) ? it.id : 'all'">
                  <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
                       stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
                       v-html="it.icon" />
                  <span>{{ it.label }}</span>
                </a>
            </div>
          </div>
          <div class="feed decisions-feed">
            <DecisionItem v-for="s in shownSignals" :key="s.id" :legs="[s]" :single="true" @reload="load()" />
            <p v-if="!shownSignals.length" class="empty">
              {{ decisions === "all" ? "No decisions recorded yet." : `Nothing ${decisions} here.` }}
            </p>
          </div>
        </section>

        <details class="card fold" :open="!!(detail?.headlines ?? []).length">
          <summary class="card-head">
            <div class="head-title">
              <h2>Headlines</h2>
              <span class="sub">{{ (detail?.headlines ?? []).length }}{{ (detail?.headlines ?? []).length ? "" : " — nothing stored" }}</span>
            </div>
          </summary>
          <div class="fold-body">
            <ul v-if="(detail?.headlines ?? []).length" class="headlines">
              <li v-for="h in detail!.headlines" :key="h.id">
                <a :href="h.url" target="_blank" rel="noreferrer noopener">{{ h.title }}</a>
                <span class="cites">{{ h.publisher ?? "" }} · {{ day(h.published_at) }}</span>
              </li>
            </ul>
            <p v-else class="empty">No headlines stored yet.</p>
          </div>
        </details>
      </div>

      <aside class="inst-side">
        <section class="card">
          <div class="card-head"><h2>Exit plan</h2></div>
          <div v-if="position && (position.stop !== null || position.target !== null)" class="plan">
            <div v-if="position.stop !== null" class="pair"><span>Sell if it falls to</span><b class="num down">{{ money(position.stop) }}</b></div>
            <div v-if="position.target !== null" class="pair"><span>Take profit at</span><b class="num up">{{ money(position.target) }}</b></div>
            <div v-if="position.stop !== null" class="pair"><span>Money at risk</span>
              <b class="num down">{{ signed((position.stop - (ltp ?? position.avg_cost)) * position.qty) }}</b></div>
            <div v-if="position.target !== null" class="pair"><span>Reward if it works</span>
              <b class="num up">{{ signed((position.target - (ltp ?? position.avg_cost)) * position.qty) }}</b></div>
            <div v-if="position.stop !== null && position.target !== null && position.stop !== (ltp ?? position.avg_cost)" class="pair">
              <span>Reward : risk</span>
              <b class="num">{{
                (Math.abs((position.target - (ltp ?? position.avg_cost)) / (position.stop - (ltp ?? position.avg_cost))) || 0).toFixed(1)
              }} : 1</b>
            </div>
          </div>
          <p v-else class="empty">
            {{ isFund ? "A fund has no intraday exit: units are allotted at NAV." : "No stop or target recorded yet." }}
          </p>
        </section>

        <section class="card">
          <div class="card-head"><h2>Profit expected</h2></div>
          <form class="target-form" @submit.prevent="saveTarget">
            <input v-model="target" name="pct" type="number" min="0" step="0.5" placeholder="default"
                   aria-label="Expected profit percent" />
            <button class="d-btn d-btn-sm" type="submit">Save</button>
          </form>
          <p class="help">Blank uses the pot's minimum; the band's maximum always wins.</p>
        </section>

        <section v-if="!isFund" class="card">
          <div class="card-head"><h2>What the rules see</h2></div>
          <div v-if="a.checks.length" class="rules">
            <div v-for="c in a.checks" :key="c.name" class="rule-row">
              <span v-html="pill(c.passed ? 'ok' : 'todo', c.passed ? 'pass' : 'block')" />
              <span>{{ c.name }} <span class="sub">{{ c.detail }}</span></span>
            </div>
          </div>
          <p v-else class="empty">Not enough data for a rule check yet.</p>
        </section>

        <section class="card">
          <div class="card-head"><h2>Manage</h2></div>
          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="refreshNow">Refresh</button>
          <p class="help" role="status">{{ refreshStatus }}</p>
          <template v-if="archived">
            <button class="d-btn d-btn-sm d-btn-primary" type="button" @click="restore">Restore</button>
            <p class="help" role="status">
              It is archived: no prices are fetched and no decisions are made. Restoring puts it back
              in the book — the fills and decisions above were never touched.
            </p>
          </template>
          <template v-else>
            <button class="d-btn d-btn-sm d-btn-ghost" type="button" :disabled="!!position" @click="archive">
              {{ archiving ? "Archive? Click again — prices cleared, fills kept" : "Archive" }}
            </button>
            <p class="help" role="status">
              {{ manageStatus || (position
                ? "An instrument you hold cannot be archived — sell it first."
                : "Archiving clears the prices and headlines; the fills and decisions stay on the Archived page.") }}
            </p>
          </template>
        </section>

        <section v-if="facts.length && !isFund" class="card">
          <div class="card-head"><h2>Indicators</h2></div>
          <div class="facts">
            <div v-for="[k, v] in facts" :key="k" class="pair"><span>{{ k }}</span><span class="num">{{ v }}</span></div>
          </div>
        </section>
      </aside>
    </div>
  </template>
</template>
