<script setup lang="ts">
/**
 * The book as a statement: what it is worth, what moved last, and what is held. Everything on the
 * page comes from the one dashboard call, which already answers 304 when nothing changed. The
 * review queue is not here — a pending decision leads to its own page.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { ChevronRight, RefreshCw } from "lucide-vue-next";
import {
    api,
    type Dashboard,
    type PositionRow,
    type SignalRow,
    type Trade,
} from "../api";
import { clock, histDate, money, units } from "../format";
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

const fills = computed(() => (data.value?.recent_trades ?? []).slice(0, 5));
const net = computed(() =>
    fills.value.reduce(
        (n, t) => n + (t.side === "sell" ? 1 : -1) * t.qty * t.price,
        0,
    ),
);

const nameOf = (symbol: string): string =>
    positions.value.find((p) => p.symbol === symbol)?.name ?? symbol;

function isToday(iso: string): boolean {
    const at = new Date(iso);
    const now = new Date();
    return (
        at.getFullYear() === now.getFullYear() &&
        at.getMonth() === now.getMonth() &&
        at.getDate() === now.getDate()
    );
}

const fillDay = (t: Trade): string =>
    isToday(t.traded_at) ? "Today" : histDate(t.traded_at);

const fillGroups = computed(() => {
    const groups: { day: string; rows: Trade[] }[] = [];
    for (const t of fills.value) {
        const day = fillDay(t);
        const last = groups[groups.length - 1];
        if (last && last.day === day) last.rows.push(t);
        else groups.push({ day, rows: [t] });
    }
    return groups;
});

const holdingMeta = (p: PositionRow): string =>
    p.class === "stock"
        ? `${p.symbol} · ${units(p.qty)} shares @ ${money(p.avg_cost)}`
        : `${units(p.qty)} units · NAV ${money(p.avg_cost)}`;

const holdingPnl = (p: PositionRow): number =>
    (p.value ?? p.qty * p.avg_cost) - p.qty * p.avg_cost;
const holdingPct = (p: PositionRow): number =>
    p.qty * p.avg_cost > 0 ? (holdingPnl(p) / (p.qty * p.avg_cost)) * 100 : 0;

/* The dashboard carries no price history, so each row's tape is one analysis read, cached for
   five minutes and capped at eight rows. */
const sparks = ref<Record<string, number[]>>({});
const sparkCache = new Map<string, { at: number; points: number[] }>();

function tapePoints(points: number[] | undefined): string {
    if (!points || points.length < 2) return "";
    const lo = Math.min(...points);
    const hi = Math.max(...points);
    const span = hi - lo || lo * 0.02 || 1;
    return points
        .map((v, i) => {
            const x = (i * 110) / (points.length - 1);
            const y = 28 - ((v - lo) / span) * 26;
            return `${x.toFixed(1)},${y.toFixed(1)}`;
        })
        .join(" ");
}

const pointsOf = (p: PositionRow): number[] | undefined =>
    sparks.value[p.symbol];

function dirOf(p: PositionRow): "up" | "down" | "flat" {
    const pts = pointsOf(p);
    if (!pts || pts.length < 2) return "flat";
    const last = pts[pts.length - 1];
    if (last > pts[0]) return "up";
    if (last < pts[0]) return "down";
    return "flat";
}

async function loadSparks(rows: PositionRow[]): Promise<void> {
    const now = Date.now();
    await Promise.all(
        rows.slice(0, 8).map(async (p) => {
            const hit = sparkCache.get(p.symbol);
            if (hit && now - hit.at < 5 * 60_000) {
                sparks.value = { ...sparks.value, [p.symbol]: hit.points };
                return;
            }
            try {
                const a = await api.analysis(p.symbol);
                const points = a.bars.slice(-60).map((b) => b.close);
                if (points.length < 2) return;
                sparkCache.set(p.symbol, { at: now, points });
                sparks.value = { ...sparks.value, [p.symbol]: points };
            } catch {
                /* a row without a tape is still a usable row */
            }
        }),
    );
}

async function load(): Promise<void> {
    busy.value = true;
    try {
        data.value = await api.dashboard(mode.value);
        error.value = "";
        void loadSparks(data.value.positions);
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
                    <h2>Recent<span class="desk-only"> fills</span></h2>
                    <span class="r"
                        >{{ fills.length }}
                        {{ fills.length === 1 ? "fill" : "fills"
                        }}<span class="desk-only">
                            · net
                            <span class="num">{{ signed(net) }}</span></span
                        ></span
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
                            <td class="num when">
                                {{ clock(t.traded_at) }}
                                <small>{{ fillDay(t) }}</small>
                            </td>
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
                    <template v-for="g in fillGroups" :key="g.day">
                        <div class="fill-day">
                            <b>{{ g.day }}</b><i></i>
                        </div>
                        <div v-for="t in g.rows" :key="t.id" class="fill-row">
                            <span class="num when">{{
                                clock(t.traded_at)
                            }}</span>
                            <span class="who">
                                <span
                                    class="pill"
                                    :class="t.side === 'buy' ? 'p-b' : 'p-s'"
                                    >{{ t.side.toUpperCase() }}</span
                                >
                                {{ nameOf(t.symbol) }}
                            </span>
                            <span class="num qty">{{ units(t.qty) }}</span>
                            <b class="num">{{ money(t.qty * t.price) }}</b>
                        </div>
                    </template>
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
                            <span class="bk-title">{{ p.name }}</span>
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
                    <span class="bk-tape">
                        <svg
                            v-if="pointsOf(p)"
                            class="bk-spark"
                            :class="dirOf(p)"
                            viewBox="0 0 110 30"
                            preserveAspectRatio="none"
                            aria-hidden="true"
                        >
                            <polyline
                                :points="tapePoints(pointsOf(p))"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="1.7"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                vector-effect="non-scaling-stroke"
                            />
                        </svg>
                        <span v-else class="bk-notape" aria-hidden="true">
                            <i></i>no tape
                        </span>
                    </span>
                    <svg
                        class="bk-trend"
                        :class="dirOf(p)"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                    >
                        <template v-if="dirOf(p) === 'up'">
                            <path d="M7 17 17 7" />
                            <path d="M7 7h10v10" />
                        </template>
                        <template v-else-if="dirOf(p) === 'down'">
                            <path d="M7 7l10 10" />
                            <path d="M17 7v10H7" />
                        </template>
                        <path v-else d="M5 12h14" />
                    </svg>
                    <div class="bk-val">
                        <b class="num">{{
                            money(p.value ?? p.qty * p.avg_cost)
                        }}</b>
                        <span v-if="p.value === null" class="muted">—</span>
                        <template v-else>
                            <span
                                class="num"
                                :class="holdingPnl(p) >= 0 ? 'up' : 'down'"
                                >{{ signed(holdingPnl(p)) }}</span
                            >
                            <span
                                class="num gap"
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
.fills .when small {
    display: block;
    font-family: var(--font-form);
    font-size: 9.5px;
    color: var(--ink-tertiary);
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
.fill-day {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 14px 3px;
}
.fill-day:first-child {
    padding-top: 2px;
}
.fill-day b {
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
}
.fill-day i {
    flex: 1;
    border-top: 1px solid var(--rule-subtle);
}
.fill-row {
    display: grid;
    grid-template-columns: 50px minmax(0, 1fr) 36px 82px;
    gap: 9px;
    align-items: center;
    padding: 7px 14px;
    border-top: 1px dotted var(--rule-subtle);
    font-size: 11.5px;
}
.fill-row > b {
    text-align: right;
    white-space: nowrap;
}
.fill-day + .fill-row {
    border-top: 0;
}
.fill-row .qty {
    text-align: right;
    color: var(--ink-secondary);
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
    grid-template-columns: minmax(0, 1.6fr) 120px 158px 18px;
    gap: 18px;
    align-items: center;
    padding: 13px 16px;
    border-top: 1px solid var(--rule-subtle);
    text-decoration: none;
    color: inherit;
}
.bk-tape {
    display: flex;
    align-items: center;
    justify-self: start;
    width: 110px;
    height: 30px;
}
.bk-notape {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 110px;
    font-size: 8.5px;
    font-weight: 500;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--ink-muted);
}
.bk-notape i {
    flex: 1;
    border-top: 1.5px dotted var(--ink-muted);
    opacity: 0.55;
}
.bk-spark {
    width: 110px;
    height: 30px;
}
.bk-spark.up,
.bk-trend.up {
    color: var(--green);
}
.bk-spark.down,
.bk-trend.down {
    color: var(--stamp-oxide);
}
.bk-trend {
    display: none;
    width: 16px;
    height: 16px;
    color: var(--ink-muted);
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
}
.bk-title {
    min-width: 0;
}
.bk-meta {
    font-size: 11px;
    color: var(--ink-tertiary);
    margin-top: 2px;
}
.bk-val {
    text-align: right;
    white-space: nowrap;
}
.bk-val b {
    display: block;
    font-size: 14px;
    font-weight: 500;
}
.bk-val span {
    font-size: 10.5px;
}
.bk-val .gap {
    margin-left: 5px;
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

/* the review queue, in one line under the statement */
.pending-strip {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
    padding: 10px 14px;
    background: var(--brand-soft);
    border-radius: var(--radius-sm);
    text-decoration: none;
    color: var(--ink);
    font-size: 12px;
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
.desk-only {
    display: inline;
}

@media (max-width: 900px) {
    .pg-top {
        position: relative;
        flex-direction: column;
        align-items: stretch;
        gap: 8px;
    }
    .pg-side {
        position: absolute;
        top: 0;
        right: 0;
    }
    .pg .pg-eyebrow,
    .pg .pg-sub {
        display: none;
    }
    .pg .pg-title {
        font-size: 26px;
        margin: 0;
        padding-right: 84px;
    }
    .only-desk {
        display: none;
    }
    .desk-only {
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
        padding: 10px 12px;
        font-size: 11.5px;
    }
    .free-mobile {
        display: inline;
    }
    .smeta > div {
        padding: 11px 14px;
    }
    .smeta b {
        font-size: 13.5px;
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
        grid-template-columns: minmax(0, 1fr) 16px 104px;
        gap: 8px;
        padding: 11px 14px;
    }
    .bk-tape {
        display: none;
    }
    .bk-trend {
        display: block;
    }
    .bk-chev {
        display: none;
    }
    .bk-name {
        font-size: 12.5px;
        align-items: flex-start;
    }
    .bk-name .tag {
        margin-top: 1px;
    }
}
</style>
