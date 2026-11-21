<script setup lang="ts">
/**
 * The book as a statement: what it is worth, what moved last, and what is held. Everything on the
 * page comes from the one dashboard call, which already answers 304 when nothing changed. The
 * review queue is not here — a pending decision leads to its own page.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { ArrowUpRight, ChevronRight, RefreshCw } from "lucide-vue-next";
import {
    api,
    type Dashboard,
    type PositionRow,
    type SignalRow,
    type Trade,
} from "../api";
import { clock, histDate, money, units } from "../format";
import { railBadges } from "../stores/rail";
import { debitOf } from "../lib/decision";
import AddInstrument from "../components/AddInstrument.vue";

const route = useRoute();
const router = useRouter();
const data = ref<Dashboard | null>(null);
const error = ref("");
const busy = ref(false);
const filter = ref<"all" | "stock" | "fund">("all");

const mode = computed(() => (route.query.mode === "trade" ? "trade" : "long"));
const isTrade = computed(() => mode.value === "trade");

const positions = computed(() => data.value?.positions ?? []);
const stocks = computed(() =>
    positions.value.filter((p) => p.class === "stock"),
);
const funds = computed(() =>
    positions.value.filter((p) => p.class !== "stock"),
);
const shown = computed(() =>
    filter.value === "all"
        ? positions.value
        : filter.value === "stock"
          ? stocks.value
          : funds.value,
);

const worth = (rows: PositionRow[]): number =>
    rows.reduce((n, p) => n + (p.value ?? p.qty * p.avg_cost), 0);
const cost = (rows: PositionRow[]): number =>
    rows.reduce((n, p) => n + p.qty * p.avg_cost, 0);

const invested = computed(() => cost(positions.value));
const value = computed(() => worth(positions.value));
const whole = computed(() => money(Math.trunc(value.value), 0));
const cents = computed(() => {
    const part = Math.round(
        Math.abs(value.value - Math.trunc(value.value)) * 100,
    );
    return String(part).padStart(2, "0");
});
const pnl = computed(() => value.value - invested.value);
const pnlPct = computed(() =>
    invested.value > 0 ? (pnl.value / invested.value) * 100 : 0,
);
const stockPct = computed(() =>
    value.value > 0 ? (worth(stocks.value) / value.value) * 100 : 0,
);

const signed = (n: number, decimals = 2): string =>
    `${n > 0 ? "+" : n < 0 ? "−" : ""}${money(Math.abs(n), decimals)}`;
const pnlTone = computed(() =>
    pnl.value > 0 ? "up" : pnl.value < 0 ? "down" : "",
);

/** A plan is one decision whatever its legs; the money it needs is the net of those legs. */
const pending = computed(() => {
    const rows = data.value?.signals ?? [];
    const groups = new Map<string, SignalRow[]>();
    for (const s of rows) {
        const key = s.plan_id ?? s.id;
        groups.set(key, [...(groups.get(key) ?? []), s]);
    }
    let needed = 0;
    for (const legs of groups.values()) needed += debitOf(legs);
    return { count: groups.size, needed };
});

const fills = computed(() => (data.value?.recent_trades ?? []).slice(0, 4));
const net = computed(() =>
    fills.value.reduce(
        (n, t) => n + (t.side === "sell" ? 1 : -1) * t.qty * t.price,
        0,
    ),
);

const nameOf = (symbol: string): string =>
    positions.value.find((p) => p.symbol === symbol)?.name ?? symbol;

/** A fill from today reads as a time; an older one needs its date or it lies. */
function fillWhen(t: Trade): string {
    const at = new Date(t.traded_at);
    const now = new Date();
    const sameDay =
        at.getFullYear() === now.getFullYear() &&
        at.getMonth() === now.getMonth() &&
        at.getDate() === now.getDate();
    return sameDay ? clock(t.traded_at) : histDate(t.traded_at);
}

const holdingMeta = (p: PositionRow): string =>
    p.class === "stock"
        ? `${p.symbol} · ${units(p.qty)} shares @ ${money(p.avg_cost)}`
        : `${units(p.qty)} units · NAV ${money(p.avg_cost)}`;

const holdingPnl = (p: PositionRow): number =>
    (p.value ?? p.qty * p.avg_cost) - p.qty * p.avg_cost;
const holdingPct = (p: PositionRow): number =>
    p.qty * p.avg_cost > 0 ? (holdingPnl(p) / (p.qty * p.avg_cost)) * 100 : 0;

async function load(): Promise<void> {
    busy.value = true;
    try {
        data.value = await api.dashboard(mode.value);
        railBadges.ledger = String(data.value.positions.length);
        error.value = "";
    } catch (e) {
        error.value = String(e);
    } finally {
        busy.value = false;
    }
}

function setMode(next: "long" | "trade"): void {
    void router.push({ path: "/", query: { mode: next } });
}

let timer: number | undefined;
onMounted(async () => {
    await load();
    timer = window.setInterval(() => void load(), 60_000);
});
onBeforeUnmount(() => window.clearInterval(timer));
watch(mode, load);
</script>

<template>
    <div class="pg">
        <div v-if="error" class="error-band">
            Something failed: {{ error }}
            <button class="d-btn d-btn-xs" type="button" @click="load()">
                Retry
            </button>
        </div>

        <template v-else-if="data">
            <header class="pg-top">
                <div class="pg-main">
                    <div class="pg-eyebrow">
                        {{
                            isTrade
                                ? "Short-term book · separate purse"
                                : "Personal book · NSE"
                        }}
                    </div>
                    <h1 class="pg-title">Book</h1>
                    <div class="pg-sub">
                        {{
                            isTrade
                                ? `${positions.length} ${positions.length === 1 ? "position" : "positions"} · as of the last poll`
                                : `${stocks.length} ${stocks.length === 1 ? "stock" : "stocks"} and ${funds.length} ${funds.length === 1 ? "fund" : "funds"} · as of the last poll`
                        }}
                    </div>
                    <div class="bk-seg">
                        <div class="dseg" role="group" aria-label="Which book">
                            <button
                                type="button"
                                :class="{ on: !isTrade }"
                                :aria-pressed="!isTrade"
                                @click="setMode('long')"
                            >
                                Long-term
                            </button>
                            <button
                                type="button"
                                :class="{ on: isTrade }"
                                :aria-pressed="isTrade"
                                @click="setMode('trade')"
                            >
                                Short-term
                            </button>
                        </div>
                    </div>
                </div>
                <div class="pg-side">
                    <div class="pg-kpis">
                        <div>
                            <span>Value</span>
                            <b class="num">{{ money(value) }}</b>
                        </div>
                        <div>
                            <span>Invested</span>
                            <b class="num">{{ money(invested) }}</b>
                        </div>
                        <div>
                            <span>P&amp;L</span>
                            <b class="num" :class="pnlTone">{{
                                signed(pnl)
                            }}</b>
                        </div>
                        <RouterLink class="pg-kpi-link" to="/decisions">
                            <span>Pending</span>
                            <b class="num"
                                >{{ pending.count
                                }}<ArrowUpRight
                                    :size="14"
                                    :stroke-width="1.75"
                                    aria-hidden="true"
                            /></b>
                        </RouterLink>
                    </div>
                    <div class="pg-acts">
                        <button
                            class="d-btn d-btn-xs"
                            type="button"
                            :disabled="busy"
                            aria-label="Refresh"
                            @click="load()"
                        >
                            <RefreshCw
                                :size="14"
                                :stroke-width="1.75"
                                aria-hidden="true"
                            />
                            <span class="acts-label">{{
                                busy ? "Refreshing…" : "Refresh"
                            }}</span>
                        </button>
                        <AddInstrument />
                    </div>
                </div>
            </header>

            <section class="statement">
                <div class="slead">
                    <div class="pg-eyebrow">Portfolio value</div>
                    <div class="giant num">
                        {{ whole }}<small>.{{ cents }}</small>
                    </div>
                    <div class="delta" :class="pnlTone">
                        {{ signed(pnl) }}
                        <span class="hint"
                            >{{ signed(pnlPct, 1) }}% on
                            {{ money(invested) }} invested</span
                        >
                    </div>
                    <div
                        class="alloc"
                        role="img"
                        :aria-label="`${stockPct.toFixed(0)}% stocks, ${(100 - stockPct).toFixed(0)}% funds`"
                    >
                        <i
                            class="stocks"
                            :style="{ width: `${stockPct}%` }"
                        ></i>
                        <i
                            class="funds"
                            :style="{ width: `${100 - stockPct}%` }"
                        ></i>
                    </div>
                    <div class="legend">
                        <span
                            ><i class="sw stocks"></i>Stocks
                            <b class="num">{{ money(worth(stocks)) }}</b></span
                        >
                        <span
                            ><i class="sw funds"></i>Funds
                            <b class="num">{{ money(worth(funds)) }}</b></span
                        >
                        <span class="free-mobile"
                            >Free
                            <b class="num">{{ money(data.available) }}</b></span
                        >
                    </div>
                </div>
                <div class="smeta">
                    <div>
                        <span>Cost basis</span
                        ><b class="num">{{ money(invested) }}</b>
                    </div>
                    <div>
                        <span>Free to deploy</span
                        ><b class="num">{{ money(data.available) }}</b>
                    </div>
                    <div>
                        <span>Positions</span
                        ><b class="num">{{ positions.length }}</b>
                    </div>
                    <div>
                        <span>Book P&amp;L</span
                        ><b class="num" :class="pnlTone">{{ signed(pnl) }}</b>
                    </div>
                </div>
            </section>

            <RouterLink to="/decisions" class="pending-strip">
                <span class="pill p-a">PENDING</span>
                <b
                    >{{ pending.count }}
                    {{ pending.count === 1 ? "decision" : "decisions" }}
                    waiting</b
                >
                <span class="num hint">{{ money(pending.needed) }}</span>
                <ChevronRight
                    :size="14"
                    :stroke-width="1.75"
                    aria-hidden="true"
                />
            </RouterLink>

            <section v-if="fills.length" class="panel">
                <div class="panel-head">
                    <h2>Recent fills</h2>
                    <span class="r"
                        >{{ fills.length }}
                        {{ fills.length === 1 ? "fill" : "fills" }} · net
                        <span class="num">{{ signed(net) }}</span></span
                    >
                </div>
                <table class="fills only-desk">
                    <thead>
                        <tr>
                            <th scope="col" class="c-when">When</th>
                            <th scope="col" class="c-side">Side</th>
                            <th scope="col">Instrument</th>
                            <th scope="col" class="r c-qty">Qty</th>
                            <th scope="col" class="r c-price">Price</th>
                            <th scope="col" class="r c-value">Value</th>
                            <th scope="col" class="r c-mode">Book</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr v-for="t in fills" :key="t.id">
                            <td class="num when">{{ fillWhen(t) }}</td>
                            <td>
                                <span
                                    class="pill"
                                    :class="t.side === 'buy' ? 'p-b' : 'p-s'"
                                    >{{ t.side.toUpperCase() }}</span
                                >
                            </td>
                            <td>
                                <b>{{ nameOf(t.symbol) }}</b>
                                <span class="sym">{{ t.symbol }}</span>
                            </td>
                            <td class="r num">{{ units(t.qty) }}</td>
                            <td class="r num">{{ money(t.price) }}</td>
                            <td class="r num">{{ money(t.qty * t.price) }}</td>
                            <td class="r hint">
                                {{
                                    t.mode === "trade"
                                        ? "short-term"
                                        : "long-term"
                                }}
                            </td>
                        </tr>
                    </tbody>
                </table>
                <div class="fill-list">
                    <div v-for="t in fills" :key="t.id" class="fill-row">
                        <span class="num when">{{ fillWhen(t) }}</span>
                        <span class="who">
                            <span
                                class="pill"
                                :class="t.side === 'buy' ? 'p-b' : 'p-s'"
                                >{{ t.side.toUpperCase() }}</span
                            >
                            {{ nameOf(t.symbol) }} {{ units(t.qty) }}
                        </span>
                        <b class="num">{{ money(t.qty * t.price) }}</b>
                    </div>
                </div>
            </section>

            <section class="panel">
                <div class="panel-head">
                    <h2>Holdings</h2>
                    <div class="mseg" role="group" aria-label="Filter holdings">
                        <button
                            type="button"
                            :class="{ on: filter === 'all' }"
                            :aria-pressed="filter === 'all'"
                            @click="filter = 'all'"
                        >
                            All {{ positions.length }}
                        </button>
                        <button
                            type="button"
                            :class="{ on: filter === 'stock' }"
                            :aria-pressed="filter === 'stock'"
                            @click="filter = 'stock'"
                        >
                            Stocks {{ stocks.length }}
                        </button>
                        <button
                            type="button"
                            :class="{ on: filter === 'fund' }"
                            :aria-pressed="filter === 'fund'"
                            @click="filter = 'fund'"
                        >
                            Funds {{ funds.length }}
                        </button>
                    </div>
                </div>

                <RouterLink
                    v-for="p in shown"
                    :key="p.id"
                    class="bk-row"
                    :to="`/i/${encodeURIComponent(p.symbol)}`"
                >
                    <div class="bk-who">
                        <div class="bk-name">
                            {{ p.name }}
                            <span
                                class="tag"
                                :class="{ fund: p.class !== 'stock' }"
                                >{{
                                    p.class === "stock" ? "STOCK" : "FUND"
                                }}</span
                            >
                        </div>
                        <div class="bk-meta">{{ holdingMeta(p) }}</div>
                    </div>
                    <div class="bk-val">
                        <b class="num">{{
                            money(p.value ?? p.qty * p.avg_cost)
                        }}</b>
                        <template v-if="p.value === null">
                            <span class="muted">—</span>
                        </template>
                        <template v-else>
                            <span
                                class="num"
                                :class="holdingPnl(p) >= 0 ? 'up' : 'down'"
                                >{{ signed(holdingPnl(p)) }}</span
                            >
                            <span
                                class="num"
                                :class="holdingPnl(p) >= 0 ? 'up' : 'down'"
                                >{{ signed(holdingPct(p), 1) }}%</span
                            >
                        </template>
                    </div>
                    <ChevronRight
                        class="bk-chev"
                        :size="16"
                        :stroke-width="1.75"
                        aria-hidden="true"
                    />
                </RouterLink>

                <p v-if="!shown.length" class="empty-note">
                    {{
                        positions.length
                            ? "Nothing in this filter."
                            : "Nothing here yet — add an instrument above."
                    }}
                </p>
            </section>
        </template>
    </div>
</template>

<style scoped>
.bk-seg {
    margin-top: 10px;
}
/* statement */
.statement {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
    background: var(--paper-raised);
    border: 1px solid var(--rule-subtle);
    border-radius: var(--radius-card);
    overflow: hidden;
    margin-bottom: 14px;
    box-shadow: var(--shadow-card);
}
.slead {
    padding: 22px 24px 18px;
    border-right: 1px solid var(--rule-subtle);
}
.giant {
    font-size: 44px;
    letter-spacing: -0.045em;
    line-height: 1.05;
    font-weight: 500;
    margin-top: 4px;
}
.delta {
    font-size: 13px;
    margin-top: 5px;
}
.delta .hint {
    margin-left: 8px;
    font-family: var(--font-form);
    color: var(--ink-tertiary);
}
.alloc {
    display: flex;
    height: 12px;
    border-radius: 6px;
    overflow: hidden;
    margin: 18px 0 9px;
    background: var(--paper-inset);
}
.alloc i {
    display: block;
    height: 100%;
}
.alloc .stocks,
.sw.stocks {
    background: var(--ink);
}
.alloc .funds,
.sw.funds {
    background: var(--fund);
}
.legend {
    display: flex;
    gap: 18px;
    font-size: 11px;
    color: var(--ink-secondary);
    flex-wrap: wrap;
}
.legend b {
    font-family: var(--font-ledger);
    color: var(--ink);
    font-weight: 500;
}
.sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    display: inline-block;
    margin-right: 6px;
}
.smeta {
    display: grid;
    grid-template-columns: 1fr 1fr;
}
.smeta > div {
    padding: 15px 22px;
    border-bottom: 1px solid var(--rule-subtle);
}
.smeta > div:nth-child(odd) {
    border-right: 1px solid var(--rule-subtle);
}
.smeta > div:nth-child(n + 3) {
    border-bottom: 0;
}
.smeta span {
    display: block;
    font-size: 9.5px;
    letter-spacing: 0.11em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.smeta b {
    font-size: 15px;
    font-weight: 500;
}
.up {
    color: var(--green);
}
.down {
    color: var(--stamp-oxide);
}

/* recent fills */
.fills {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    table-layout: fixed;
}
.fills th {
    font-size: 9.5px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    text-align: left;
    padding: 8px 16px;
    background: var(--paper);
    font-weight: 700;
    border-top: 1px solid var(--rule-subtle);
}
.fills td {
    padding: 10px 16px;
    border-top: 1px solid var(--rule-subtle);
    vertical-align: middle;
    overflow-wrap: anywhere;
}
.fills tbody tr:hover {
    background: var(--paper);
}
.fills .r {
    text-align: right;
}
.fills .when {
    font-size: 12px;
    color: var(--ink-secondary);
}
.fills .c-when {
    width: 84px;
}
.fills .c-side {
    width: 78px;
}
.fills .c-qty {
    width: 74px;
}
.fills .c-price {
    width: 108px;
}
.fills .c-value {
    width: 126px;
}
.fills .c-mode {
    width: 96px;
}
.sym {
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--ink-tertiary);
    margin-left: 6px;
}
.fill-list {
    display: none;
}
.fill-row {
    display: grid;
    grid-template-columns: 58px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 9px 14px;
    border-top: 1px solid var(--rule-subtle);
    font-size: 11.5px;
}
.fill-row:first-child {
    border-top: 0;
}
.fill-row .who {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
}
.fill-row .when {
    color: var(--ink-tertiary);
}

/* holdings */
.bk-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 210px 18px;
    gap: 18px;
    align-items: center;
    padding: 13px 16px;
    border-top: 1px solid var(--rule-subtle);
    text-decoration: none;
    color: inherit;
}
.bk-row:hover,
.bk-row:focus-visible {
    background: var(--paper);
}
.bk-name {
    font-weight: 640;
    font-size: 13.5px;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
}
.bk-meta {
    font-size: 11px;
    color: var(--ink-tertiary);
    margin-top: 2px;
}
.bk-val {
    text-align: right;
    display: grid;
    justify-items: end;
}
.bk-val b {
    font-size: 14px;
    font-weight: 500;
}
.bk-val span {
    font-size: 10.5px;
}
.bk-val .muted {
    color: var(--ink-muted);
    font-family: var(--font-ledger);
    font-size: 11px;
}
.bk-chev {
    color: var(--ink-tertiary);
}
.empty-note {
    padding: 18px 14px;
    margin: 0;
    text-align: center;
    font-size: 12px;
    color: var(--ink-tertiary);
}

/* the phone's review strip */
.pending-strip {
    display: none;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
    padding: 10px 12px;
    background: var(--brand-soft);
    border-radius: var(--radius-sm);
    text-decoration: none;
    color: var(--ink);
    font-size: 11.5px;
}
.pending-strip b {
    font-weight: 650;
}
.pending-strip .hint {
    margin-left: auto;
    color: var(--brand);
}
.pending-strip svg {
    color: var(--brand);
}
.free-mobile {
    display: none;
}

@media (max-width: 900px) {
    /* The header dissolves so its actions can be ordered below the statement. */
    .pg {
        display: flex;
        flex-direction: column;
    }
    .pg-top,
    .pg-side {
        display: contents;
    }
    .pg-main {
        order: 1;
        border-bottom: 1px solid var(--rule-subtle);
        padding-bottom: 14px;
        margin-bottom: 14px;
    }
    .pg .pg-eyebrow,
    .pg .pg-sub {
        display: none;
    }
    .pg .pg-title {
        font-size: 26px;
        margin: 0;
    }
    .pg-kpis,
    .smeta,
    .only-desk {
        display: none;
    }
    .statement {
        order: 2;
    }
    .pg-acts {
        order: 3;
        justify-content: flex-end;
        margin-bottom: 12px;
    }
    .pg-acts :deep(.acts-label) {
        display: none;
    }
    .pg-acts .d-btn {
        min-width: 38px;
        justify-content: center;
    }
    .pending-strip {
        order: 4;
        display: flex;
    }
    .panel {
        order: 5;
    }
    .free-mobile {
        display: inline;
    }
    .statement {
        grid-template-columns: minmax(0, 1fr);
    }
    .slead {
        padding: 14px 14px 12px;
        border-right: 0;
    }
    .giant {
        font-size: 34px;
        margin-top: 2px;
    }
    .delta {
        font-size: 11.5px;
    }
    .alloc {
        margin: 10px 0 8px;
        height: 10px;
    }
    .legend {
        gap: 14px;
        font-size: 10.5px;
    }
    .statement {
        margin-bottom: 10px;
    }
    .fill-list {
        display: block;
    }
    .bk-row {
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 10px;
        padding: 11px 14px;
    }
    .bk-chev {
        display: none;
    }
    .bk-name {
        font-size: 12.5px;
    }
}
</style>
