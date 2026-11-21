<script setup lang="ts">
/**
 * One instrument: the tape first, then what the position is worth, then what was done about it.
 * Archive and delete sit at the very end of the record — a thumb cannot reach them by accident.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { Plus, RefreshCw } from "lucide-vue-next";
import {
    api,
    type Analysis,
    type InstrumentDetail,
    type SignalRow,
    type Trade,
} from "../api";
import { clock, day, dayTime, money, today, units } from "../format";
import DeleteInstrument from "../components/DeleteInstrument.vue";
import PriceChart from "../components/PriceChart.vue";

const route = useRoute();
const router = useRouter();
const analysis = ref<Analysis | null>(null);
const detail = ref<InstrumentDetail | null>(null);
const trades = ref<Trade[]>([]);
const error = ref("");

type Ledger = Analysis & { trades: Trade[] };

const symbol = computed(() => String(route.params.symbol ?? ""));

const media = window.matchMedia("(max-width: 900px)");
const compact = ref(media.matches);
const onMedia = (e: MediaQueryListEvent): void => {
    compact.value = e.matches;
};

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

onMounted(() => {
    media.addEventListener("change", onMedia);
    void load();
});
onBeforeUnmount(() => media.removeEventListener("change", onMedia));
watch(symbol, load);

const a = computed(() => analysis.value);
const inst = computed(() => a.value?.instrument);
const position = computed(() => a.value?.position ?? null);
const klass = computed(() => inst.value?.class ?? "stock");
const isFund = computed(() => klass.value === "mutual_fund");
const archived = computed(() => !inst.value?.active);

const ltp = computed(() =>
    a.value?.bars.length
        ? a.value.bars[a.value.bars.length - 1].close
        : (position.value?.avg_cost ?? null),
);
const prevClose = computed(() =>
    a.value && a.value.bars.length > 1
        ? a.value.bars[a.value.bars.length - 2].close
        : null,
);
const priceTime = computed(() => {
    const bars = a.value?.bars ?? [];
    return bars.length ? clock(bars[bars.length - 1].ts) : "";
});
const change = computed(() =>
    ltp.value !== null && prevClose.value !== null
        ? ltp.value - prevClose.value
        : null,
);
const invested = computed(() =>
    position.value ? position.value.qty * position.value.avg_cost : null,
);
const value = computed(() =>
    position.value && ltp.value !== null
        ? position.value.qty * ltp.value
        : null,
);
const unrealised = computed(() =>
    invested.value !== null && value.value !== null
        ? value.value - invested.value
        : null,
);
const costPct = computed(() =>
    unrealised.value !== null && invested.value
        ? (unrealised.value / invested.value) * 100
        : null,
);
const realised = computed(() =>
    trades.value.reduce((n, t) => n + (t.realised ?? 0), 0),
);
const closedOut = computed(
    () => trades.value.length > 0 && realised.value !== 0,
);
const sign = (n: number): string => (n >= 0 ? "+" : "−");
const signed = (n: number): string => `${sign(n)}${money(Math.abs(n))}`;
const pct = (n: number): string => `${sign(n)}${Math.abs(n).toFixed(1)}%`;
const tone = (n: number | null): string =>
    n === null || n === 0 ? "" : n > 0 ? "up" : "down";

const signals = computed<SignalRow[]>(() => detail.value?.signals ?? []);
const pending = computed(
    () => signals.value.filter((s) => s.status === "pending").length,
);

const expected = computed(() => {
    const p = position.value;
    if (!p || p.target === null) return null;
    return (p.target - p.avg_cost) * p.qty;
});

const rr = computed(() => {
    const p = position.value;
    if (!p || p.stop === null || p.target === null || ltp.value === null)
        return null;
    const risk = ltp.value - p.stop;
    if (risk <= 0) return null;
    return (p.target - ltp.value) / risk;
});

const blocked = computed(() =>
    (a.value?.checks ?? []).filter((c) => !c.passed),
);
const ruleNote = computed(() =>
    !a.value?.checks.length
        ? "no rule check yet"
        : blocked.value.length
          ? `${blocked.value.length} blocking`
          : "all clear",
);

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

/* ---- recording a fill ---------------------------------------------------- */
const recording = ref(false);
const fillDialog = ref<HTMLDialogElement | null>(null);
const side = ref<"buy" | "sell">("buy");
const fill = ref({ qty: "", price: "", at: today(), mode: "long" });

watch(recording, (open) => {
    const d = fillDialog.value;
    if (!d) return;
    if (open) {
        tradeStatus.value = "";
        if (!d.open) d.showModal();
    } else if (d.open) {
        d.close();
    }
});

const heldQty = computed(() => position.value?.qty ?? 0);
const oversell = computed(
    () => side.value === "sell" && Number(fill.value.qty) > heldQty.value,
);
const fillValue = computed(() => {
    const qty = Number(fill.value.qty);
    const price = Number(fill.value.price);
    return qty > 0 && price > 0 ? qty * price : null;
});
const tradeStatus = ref("");

async function record(): Promise<void> {
    tradeStatus.value = "Recording…";
    try {
        await api.recordTrade(
            symbol.value,
            side.value,
            Number(fill.value.qty),
            Number(fill.value.price),
            fill.value.at,
            fill.value.mode as "long" | "trade",
        );
        recording.value = false;
        fill.value = { qty: "", price: "", at: today(), mode: "long" };
        tradeStatus.value = "";
        await load();
    } catch (e) {
        tradeStatus.value = String(e);
    }
}

/* ---- correcting and deleting a fill -------------------------------------- */
const editing = ref<string | null>(null);
const edit = ref({ qty: "", price: "", at: "" });
const fillStatus = ref("");
const armed = ref<string | null>(null);

function openEdit(t: Trade): void {
    editing.value = t.id;
    edit.value = {
        qty: String(t.qty),
        price: String(t.price),
        at: t.traded_at.slice(0, 10),
    };
    fillStatus.value = "";
}

async function saveEdit(id: string): Promise<void> {
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

async function deleteFill(id: string): Promise<void> {
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
const refreshStatus = ref("");
const manageStatus = ref("");
const archiving = ref(false);

watch(inst, (i) => {
    target.value =
        i?.target_profit_pct == null ? "" : String(i.target_profit_pct);
});

async function saveTarget(): Promise<void> {
    const raw = target.value.trim();
    try {
        await api.setTargetProfit(
            symbol.value,
            raw === "" ? null : Number(raw),
        );
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
        manageStatus.value =
            "Archiving clears prices and headlines; fills and decisions stay.";
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

const gone = (): void => void router.push("/archived");
</script>

<template>
    <div v-if="error" class="error-band">
        Something failed: {{ error }}
        <button class="d-btn d-btn-xs" type="button" @click="load()">
            Retry
        </button>
    </div>

    <div v-else-if="a && inst" class="pg dossier">
        <RouterLink :to="archived ? '/archived' : '/'" class="back"
            >← {{ archived ? "Archived" : "Book" }}</RouterLink
        >

        <header class="pg-top">
            <div class="pg-main">
                <div class="pg-eyebrow">
                    {{ isFund ? "Fund · AMFI" : "Stock · NSE"
                    }}{{ archived ? " · archived" : "" }}
                </div>
                <h1 class="pg-title">{{ inst.name }}</h1>
                <div class="pg-sub">
                    <span class="sym">{{ inst.symbol }}</span>
                    <template v-if="position">
                        · {{ units(position.qty) }}
                        {{ isFund ? "units" : "shares" }} · average
                        {{ money(position.avg_cost) }}</template
                    >
                    <template v-if="priceTime">
                        · {{ isFund ? "last NAV" : "last price" }}
                        {{ priceTime }} IST</template
                    >
                </div>
            </div>
            <div class="pg-side">
                <div class="price">
                    <b class="num">{{ ltp === null ? "—" : money(ltp) }}</b>
                    <span class="delta">
                        <span
                            v-if="change !== null"
                            class="num"
                            :class="tone(change)"
                        >
                            {{ signed(change) }} <em>today</em>
                        </span>
                        <span v-if="costPct !== null" class="num hint"
                            >{{ pct(costPct) }} on cost</span
                        >
                    </span>
                </div>
                <div class="pg-acts">
                    <button
                        class="d-btn d-btn-xs"
                        type="button"
                        :disabled="archived"
                        aria-label="Refresh this instrument"
                        @click="refreshNow"
                    >
                        <RefreshCw
                            :size="14"
                            :stroke-width="1.75"
                            aria-hidden="true"
                        />
                        <span class="acts-label">Refresh</span>
                    </button>
                    <button
                        class="d-btn d-btn-xs d-btn-primary record"
                        type="button"
                        :disabled="archived"
                        :aria-expanded="recording"
                        @click="recording = !recording"
                    >
                        <Plus
                            :size="14"
                            :stroke-width="1.75"
                            aria-hidden="true"
                        />
                        <span class="acts-label">Record a fill</span>
                    </button>
                </div>
                <p v-if="refreshStatus" class="hint" role="status">
                    {{ refreshStatus }}
                </p>
            </div>
        </header>

        <section class="panel hero">
            <div class="panel-head">
                <h2>{{ isFund ? "NAV" : "Price" }}</h2>
                <span class="r">
                    {{ a.bars.length }}
                    {{ isFund ? "NAV points" : "sessions" }} ·
                    {{ isFund ? "units are allotted at NAV" : "daily close" }}
                </span>
            </div>
            <PriceChart :analysis="a" :compact="compact" />
            <div v-if="facts.length && !isFund" class="facts">
                <span v-for="[k, v] in facts" :key="k"
                    ><em>{{ k }}</em> <b class="num">{{ v }}</b></span
                >
            </div>
        </section>

        <section class="band">
            <div>
                <h3>Position</h3>
                <div class="kv2">
                    <span>Shares</span
                    ><b class="num">{{
                        position ? units(position.qty) : "—"
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Average cost</span
                    ><b class="num">{{
                        position ? money(position.avg_cost) : "—"
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Invested</span
                    ><b class="num">{{
                        invested === null ? "—" : money(invested)
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Market value</span
                    ><b class="num">{{
                        value === null ? "—" : money(value)
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Unrealised</span>
                    <b class="num" :class="tone(unrealised)">{{
                        unrealised === null ? "—" : signed(unrealised)
                    }}</b>
                </div>
                <div class="kv2">
                    <span>On cost</span>
                    <b class="num" :class="tone(costPct)">{{
                        costPct === null ? "—" : pct(costPct)
                    }}</b>
                </div>
                <div class="kv2 split">
                    <span>Profit expected</span>
                    <b class="num" :class="tone(expected)">{{
                        expected === null ? "—" : signed(expected)
                    }}</b>
                </div>
                <form class="target" @submit.prevent="saveTarget">
                    <label>
                        <span>Profit target %</span>
                        <span class="pct">
                            <input
                                v-model="target"
                                name="pct"
                                type="number"
                                min="0"
                                step="0.5"
                                placeholder="pot default"
                                aria-label="Expected profit percent"
                            />
                            <i aria-hidden="true">%</i>
                        </span>
                    </label>
                    <button class="d-btn d-btn-xs" type="submit">Save</button>
                </form>
                <p class="hint">
                    Blank uses the pot's minimum; the band's maximum always
                    wins.
                </p>
            </div>

            <div>
                <h3>The plan</h3>
                <div class="kv2">
                    <span>Stop</span
                    ><b class="num">{{
                        position?.stop == null ? "—" : money(position.stop)
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Target</span
                    ><b class="num">{{
                        position?.target == null ? "—" : money(position.target)
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Reward : risk</span>
                    <b class="num">{{
                        rr === null ? "—" : `${rr.toFixed(1)} : 1`
                    }}</b>
                </div>
                <div class="kv2">
                    <span>Price data</span
                    ><b class="num">{{ a.bar_count }} / 200 bars</b>
                </div>
                <div class="kv2">
                    <span>Rule check</span>
                    <b :class="blocked.length ? 'down' : ''">{{ ruleNote }}</b>
                </div>
                <p v-if="blocked.length" class="notice">
                    <span aria-hidden="true">!</span>{{ blocked[0].detail }}
                </p>
            </div>

            <div>
                <h3>Headlines</h3>
                <template v-if="(detail?.headlines ?? []).length">
                    <a
                        v-for="h in (detail?.headlines ?? []).slice(0, 5)"
                        :key="h.id"
                        class="file"
                        :href="h.url"
                        target="_blank"
                        rel="noreferrer noopener"
                        >{{ h.title
                        }}<span class="src"
                            >{{ h.publisher ?? "—" }} ·
                            {{ day(h.published_at) }}</span
                        ></a
                    >
                </template>
                <p v-else class="hint">
                    Nothing stored yet — Refresh fetches headlines.
                </p>
                <div v-if="(detail?.headlines ?? []).length > 5" class="hint">
                    +{{ (detail?.headlines ?? []).length - 5 }} more stored
                </div>
            </div>
        </section>

        <section class="panel">
            <div class="panel-head">
                <h2>Fills</h2>
                <span class="r">
                    {{ trades.length }}
                    {{ trades.length === 1 ? "fill" : "fills" }} ·
                    {{
                        position
                            ? `${units(position.qty)} held`
                            : "nothing held"
                    }}
                </span>
            </div>

            <table v-if="trades.length" class="fills only-desk">
                <thead>
                    <tr>
                        <th scope="col" class="c-when">Date</th>
                        <th scope="col" class="c-side">Side</th>
                        <th scope="col" class="r c-qty">Qty</th>
                        <th scope="col" class="r c-price">Price</th>
                        <th scope="col" class="r c-value">Value</th>
                        <th scope="col" class="r c-vs hide-sm">vs close</th>
                        <th scope="col" class="r c-mode hide-sm">Book</th>
                        <th scope="col" class="c-act"></th>
                    </tr>
                </thead>
                <tbody>
                    <template v-for="t in trades" :key="t.id">
                        <tr>
                            <td class="num when">{{ dayTime(t.traded_at) }}</td>
                            <td>
                                <span
                                    class="pill"
                                    :class="t.side === 'buy' ? 'p-b' : 'p-s'"
                                    >{{ t.side.toUpperCase() }}</span
                                >
                            </td>
                            <td class="r num">{{ units(t.qty) }}</td>
                            <td class="r num">{{ money(t.price) }}</td>
                            <td class="r num">{{ money(t.qty * t.price) }}</td>
                            <td class="r num hide-sm" :class="vsClose(t).tone">
                                {{ vsClose(t).text }}
                            </td>
                            <td class="r hint hide-sm">
                                {{
                                    t.mode === "trade"
                                        ? "short-term"
                                        : "long-term"
                                }}
                            </td>
                            <td class="r">
                                <button
                                    class="link"
                                    type="button"
                                    :aria-expanded="editing === t.id"
                                    @click="openEdit(t)"
                                >
                                    Edit
                                </button>
                            </td>
                        </tr>
                        <tr v-if="editing === t.id">
                            <td colspan="8">
                                <form
                                    class="edit-form"
                                    @submit.prevent="saveEdit(t.id)"
                                >
                                    <label class="fld"
                                        ><span>Quantity</span
                                        ><input
                                            v-model="edit.qty"
                                            name="qty"
                                            type="number"
                                            min="0"
                                            step="any"
                                    /></label>
                                    <label class="fld"
                                        ><span>Price</span
                                        ><input
                                            v-model="edit.price"
                                            name="price"
                                            type="number"
                                            min="0"
                                            step="any"
                                    /></label>
                                    <label class="fld"
                                        ><span>Date</span
                                        ><input
                                            v-model="edit.at"
                                            name="at"
                                            type="date"
                                            :max="today()"
                                    /></label>
                                    <button
                                        class="d-btn d-btn-xs d-btn-primary"
                                        type="submit"
                                    >
                                        Save
                                    </button>
                                    <button
                                        class="d-btn d-btn-xs"
                                        type="button"
                                        @click="editing = null"
                                    >
                                        Cancel
                                    </button>
                                    <button
                                        class="d-btn d-btn-xs danger"
                                        type="button"
                                        :data-armed="armed === t.id"
                                        @click="deleteFill(t.id)"
                                    >
                                        {{
                                            armed === t.id
                                                ? "Delete? Click again"
                                                : "Delete fill"
                                        }}
                                    </button>
                                    <span
                                        v-if="fillStatus"
                                        class="hint"
                                        role="status"
                                        >{{ fillStatus }}</span
                                    >
                                </form>
                            </td>
                        </tr>
                    </template>
                </tbody>
            </table>
            <p v-else class="empty-note">No fills recorded yet.</p>
            <div v-if="trades.length" class="rows only-mobile">
                <div v-for="t in trades" :key="t.id" class="row">
                    <span class="num when">{{ day(t.traded_at) }}</span>
                    <span class="who">
                        <span
                            class="pill"
                            :class="t.side === 'buy' ? 'p-b' : 'p-s'"
                            >{{ t.side.toUpperCase() }}</span
                        >
                        {{ units(t.qty) }} @ {{ money(t.price) }}
                    </span>
                    <b class="num">{{ money(t.qty * t.price) }}</b>
                </div>
            </div>
            <div class="panel-foot">
                <span class="hint"
                    >A fill moves the position, the trade and the cash together,
                    in one transaction.</span
                >
            </div>
        </section>

        <section class="panel">
            <div class="panel-head">
                <h2>Decisions</h2>
                <span class="r"
                    >{{ signals.length }} recorded · {{ pending }} pending</span
                >
            </div>
            <table v-if="signals.length" class="dec only-desk">
                <thead>
                    <tr>
                        <th scope="col" class="c-when">When</th>
                        <th scope="col" class="c-status">Status</th>
                        <th scope="col" class="c-side">Side</th>
                        <th scope="col" class="r">Qty @ price</th>
                        <th scope="col" class="r hide-sm">Stop</th>
                        <th scope="col" class="r hide-sm">Target</th>
                        <th scope="col" class="r hide-sm">Plan</th>
                    </tr>
                </thead>
                <tbody>
                    <tr v-for="s in signals" :key="s.id">
                        <td class="num when">{{ dayTime(s.created_at) }}</td>
                        <td>
                            <span
                                class="pill"
                                :class="
                                    s.status === 'approved'
                                        ? 'p-b'
                                        : s.status === 'rejected'
                                          ? 'p-n'
                                          : 'p-a'
                                "
                            >
                                {{ s.status.toUpperCase() }}
                            </span>
                        </td>
                        <td>
                            <span
                                class="pill"
                                :class="s.action === 'SELL' ? 'p-s' : 'p-b'"
                                >{{ s.action }}</span
                            >
                        </td>
                        <td class="r num">
                            {{
                                s.amount !== null
                                    ? money(s.amount)
                                    : `${s.qty === null ? "—" : units(s.qty)} @ ${s.entry === null ? "—" : money(s.entry)}`
                            }}
                        </td>
                        <td class="r num hide-sm">
                            {{ s.stop === null ? "—" : money(s.stop) }}
                        </td>
                        <td class="r num hide-sm">
                            {{ s.target === null ? "—" : money(s.target) }}
                        </td>
                        <td class="r sym hide-sm">
                            {{ (s.plan_id ?? "").slice(-6) || "—" }}
                        </td>
                    </tr>
                </tbody>
            </table>
            <p v-else class="empty-note">
                No decisions recorded for this instrument yet.
            </p>
            <div v-if="signals.length" class="rows only-mobile">
                <div v-for="s in signals" :key="s.id" class="row">
                    <span class="num when">{{ clock(s.created_at) }}</span>
                    <span class="who">
                        <span
                            class="pill"
                            :class="
                                s.status === 'approved'
                                    ? 'p-b'
                                    : s.status === 'rejected'
                                      ? 'p-n'
                                      : 'p-a'
                            "
                            >{{ s.status.toUpperCase() }}</span
                        >
                        {{
                            s.amount !== null
                                ? money(s.amount)
                                : `${s.qty === null ? "—" : units(s.qty)} @ ${s.entry === null ? "—" : money(s.entry)}`
                        }}
                    </span>
                    <span class="num sym">{{
                        (s.plan_id ?? "").slice(-6) || "—"
                    }}</span>
                </div>
            </div>
            <div class="panel-foot">
                <span class="hint"
                    >A pending leg is signed off in Decisions.</span
                >
                <RouterLink to="/decisions" class="link"
                    >Open decisions</RouterLink
                >
            </div>
        </section>

        <section class="panel">
            <div class="panel-head">
                <h2>Archive or delete</h2>
                <span class="r">at the end of the record</span>
            </div>
            <div class="manage">
                <p class="hint">
                    {{
                        archived
                            ? "This record is archived: no prices are fetched and no decisions are made. Restoring puts it back in the book — the fills above were never touched."
                            : `Archiving keeps the fills and clears the prices. Deleting removes the record and returns its cash to the pot.${
                                  position
                                      ? " Both are refused while you hold shares — sell it flat first."
                                      : ""
                              }`
                    }}
                </p>
                <div class="manage-acts">
                    <template v-if="archived">
                        <button
                            class="d-btn d-btn-xs d-btn-primary"
                            type="button"
                            @click="restore"
                        >
                            Restore
                        </button>
                    </template>
                    <button
                        v-else
                        class="d-btn d-btn-xs"
                        type="button"
                        :disabled="!!position"
                        @click="archive"
                    >
                        {{ archiving ? "Archive? Click again" : "Archive" }}
                    </button>
                    <DeleteInstrument
                        :symbol="symbol"
                        :name="inst.name"
                        :disabled="!!position"
                        @deleted="gone"
                    />
                </div>
            </div>
            <p v-if="manageStatus" class="panel-foot hint" role="status">
                {{ manageStatus }}
            </p>
        </section>

        <dialog
            ref="fillDialog"
            class="fill-dialog"
            aria-labelledby="fill-title"
            @close="recording = false"
        >
            <form class="fill-sheet" @submit.prevent="record">
                <header class="fill-head">
                    <div>
                        <h2 id="fill-title">Record a fill</h2>
                        <p>
                            {{ symbol }} · {{ inst.name }} ·
                            {{
                                position
                                    ? `you hold ${units(position.qty)} ${isFund ? "units" : "shares"}`
                                    : "nothing held"
                            }}
                        </p>
                    </div>
                    <span class="kbd">esc</span>
                </header>
                <div class="fill-body">
                    <div
                        class="dseg"
                        role="radiogroup"
                        aria-label="Buy or sell"
                    >
                        <button
                            type="button"
                            :class="{ on: side === 'buy' }"
                            :aria-pressed="side === 'buy'"
                            @click="side = 'buy'"
                        >
                            Buy
                        </button>
                        <button
                            type="button"
                            :class="{ on: side === 'sell' }"
                            :aria-pressed="side === 'sell'"
                            @click="side = 'sell'"
                        >
                            Sell
                        </button>
                    </div>
                    <div class="fill-grid">
                        <label class="fld">
                            <span>{{ isFund ? "Units" : "Quantity" }}</span>
                            <input
                                v-model="fill.qty"
                                name="qty"
                                type="number"
                                min="0"
                                step="any"
                                required
                                :placeholder="
                                    position
                                        ? `held ${units(position.qty)}`
                                        : isFund
                                          ? 'Units'
                                          : 'Qty'
                                "
                            />
                            <em
                                v-if="side === 'sell' && position"
                                class="fld-hint"
                                >max {{ units(position.qty) }}
                                {{ isFund ? "units" : "shares" }}</em
                            >
                        </label>
                        <label class="fld">
                            <span>{{ isFund ? "NAV" : "Price" }}</span>
                            <input
                                v-model="fill.price"
                                name="price"
                                type="number"
                                min="0"
                                step="any"
                                required
                                :placeholder="isFund ? 'NAV' : 'Price'"
                            />
                        </label>
                        <label class="fld">
                            <span>Date</span>
                            <input
                                v-model="fill.at"
                                name="at"
                                type="date"
                                :max="today()"
                            />
                        </label>
                        <label v-if="klass === 'stock'" class="fld">
                            <span>Book</span>
                            <select v-model="fill.mode" name="mode">
                                <option value="long">Long-term</option>
                                <option value="trade">Short-term</option>
                            </select>
                        </label>
                    </div>
                    <p v-if="oversell" class="fill-err" role="alert">
                        <span aria-hidden="true">!</span>
                        You hold {{ units(position?.qty ?? 0) }}
                        {{ isFund ? "units" : "shares" }} — a sell cannot be
                        more than that.
                    </p>
                    <p v-if="fillValue !== null" class="fill-sum">
                        <template v-if="side === 'sell' && position">
                            <span>Available</span>
                            <b>{{ units(position.qty) }}</b>
                            <span>·</span>
                            <span
                                >Proceeds at
                                {{ money(Number(fill.price)) }}</span
                            >
                        </template>
                        <template v-else>
                            <span>Cost at {{ money(Number(fill.price)) }}</span>
                        </template>
                        <b>{{ money(fillValue) }}</b>
                    </p>
                </div>
                <footer class="fill-foot">
                    <span v-if="tradeStatus" class="hint" role="status">{{
                        tradeStatus
                    }}</span>
                    <button
                        class="d-btn d-btn-sm d-btn-ghost"
                        type="button"
                        @click="recording = false"
                    >
                        Cancel
                    </button>
                    <button
                        class="d-btn d-btn-sm d-btn-primary"
                        type="submit"
                        :disabled="oversell || !fill.qty || !fill.price"
                    >
                        {{
                            tradeStatus === "Recording…"
                                ? "Recording…"
                                : "Record fill"
                        }}
                    </button>
                </footer>
            </form>
        </dialog>

        <div class="actbar">
            <button
                class="d-btn d-btn-xs"
                type="button"
                :disabled="archived"
                @click="refreshNow"
            >
                <RefreshCw
                    :size="14"
                    :stroke-width="1.75"
                    aria-hidden="true"
                />Refresh
            </button>
            <button
                class="d-btn d-btn-xs d-btn-primary grow"
                type="button"
                :disabled="archived"
                :aria-expanded="recording"
                @click="recording = !recording"
            >
                <Plus
                    :size="14"
                    :stroke-width="1.75"
                    aria-hidden="true"
                />Record a fill
            </button>
        </div>
    </div>
</template>

<style scoped>
.back {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 10px;
    color: var(--ink-secondary);
    font-size: 12px;
    text-decoration: none;
}
.back:hover {
    color: var(--brand);
}
.price {
    display: grid;
    justify-items: end;
    gap: 2px;
}
.price b {
    font-family: var(--font-ledger);
    font-size: 36px;
    font-weight: 500;
    letter-spacing: -0.04em;
    line-height: 1;
}
.price span {
    font-size: 12px;
}
.price .delta {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 12px;
}
.price .hint {
    font-size: 11.5px;
    color: var(--ink-tertiary);
}
.price em {
    font-style: normal;
    color: var(--ink-tertiary);
}
.record {
    min-width: 168px;
    justify-content: center;
}
.hero {
    margin-top: 4px;
}
.hero :deep(.chart) {
    margin: 0;
    padding: 4px 8px 0;
}
.hero :deep(.chart svg) {
    width: 100%;
    height: auto;
    display: block;
}
.facts {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    padding: 8px 16px 12px;
    border-top: 1px solid var(--rule-subtle);
    font-size: 10.5px;
    color: var(--ink-tertiary);
}
.facts em {
    font-style: normal;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 700;
    font-size: 9px;
    margin-right: 3px;
}
.facts b {
    color: var(--ink-secondary);
    font-weight: 500;
}

/* position / plan / headlines */
.band {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px;
    margin-bottom: 14px;
}
.band > div {
    background: var(--paper-raised);
    border: 1px solid var(--rule-subtle);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    padding: 14px 16px;
}
.band h3 {
    margin: 0 0 8px;
    font-size: 9.5px;
    letter-spacing: 0.11em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.kv2 {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    padding: 3px 0;
    font-size: 11.5px;
    color: var(--ink-secondary);
}
.kv2 b {
    font-weight: 500;
    color: var(--ink);
}
.kv2.split {
    margin-top: 8px;
    border-top: 1px solid var(--rule-subtle);
    padding-top: 8px;
}
.file {
    display: block;
    padding: 7px 0;
    border-bottom: 1px dotted var(--rule-subtle);
    font-size: 12px;
    color: var(--ink);
    text-decoration: none;
    line-height: 1.4;
}
.file:hover {
    color: var(--brand);
}
.file .src {
    display: block;
    font-size: 9.5px;
    font-family: var(--font-ledger);
    color: var(--ink-tertiary);
}
.target {
    display: flex;
    align-items: end;
    gap: 6px;
    margin-top: 8px;
}
.target label {
    display: grid;
    gap: 3px;
    flex: 1;
    min-width: 0;
}
.target label span {
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.notice {
    display: flex;
    gap: 7px;
    align-items: flex-start;
    margin: 9px 0 0;
    padding: 8px 10px;
    background: var(--stamp-oxide-bg);
    color: var(--stamp-oxide);
    border-radius: var(--radius-sm);
    font-size: 10.5px;
    line-height: 1.4;
}
.notice span {
    font-weight: 700;
}
.target input {
    height: 30px;
    min-height: 30px;
    padding: 0 8px;
    padding-right: 24px;
    font-family: var(--font-ledger);
    font-size: 12.5px;
    border: 1px solid var(--rule-strong);
    border-radius: var(--radius-sm);
    background: var(--paper-raised);
    color: var(--ink);
    width: 100%;
}
.target input::-webkit-outer-spin-button,
.target input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
}
.target .pct {
    position: relative;
    display: block;
}
.target .pct i {
    position: absolute;
    right: 9px;
    top: 50%;
    transform: translateY(-50%);
    font: 500 11px var(--font-ledger);
    font-style: normal;
    color: var(--ink-tertiary);
    pointer-events: none;
}
.hint {
    font-size: 11px;
    color: var(--ink-tertiary);
}
.up {
    color: var(--green);
}
.down {
    color: var(--stamp-oxide);
}

/* fills and decisions */
.fills,
.dec {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    table-layout: fixed;
}
.fills th,
.dec th {
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
.fills td,
.dec td {
    padding: 9px 16px;
    border-top: 1px solid var(--rule-subtle);
    vertical-align: middle;
    overflow-wrap: anywhere;
}
.fills tbody tr:hover,
.dec tbody tr:hover {
    background: var(--paper);
}
.fills .r,
.dec .r {
    text-align: right;
}
.fills .when,
.dec .when {
    font-size: 11.5px;
    color: var(--ink-secondary);
    white-space: nowrap;
}
.fills .c-when {
    width: 128px;
}
.fills .c-side {
    width: 76px;
}
.fills .c-qty {
    width: 70px;
}
.fills .c-price,
.fills .c-vs {
    width: 104px;
}
.fills .c-value {
    width: 124px;
}
.fills .c-mode {
    width: 96px;
}
.fills .c-act {
    width: 66px;
}
.dec .c-when {
    width: 128px;
}
.dec .c-status,
.dec .c-side {
    width: 92px;
}
.link {
    border: 0;
    background: none;
    font: inherit;
    font-size: 11.5px;
    color: var(--brand);
    font-weight: 600;
    cursor: pointer;
    padding: 0;
    text-decoration: none;
}
.link:hover {
    text-decoration: underline;
}
.edit-form {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: end;
    padding: 4px 0 8px;
}

/* Recording a fill is the board's modal: one dialog over the record, not a box inside it. */
.fill-dialog {
    width: min(560px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--rule-strong);
    border-radius: var(--radius-lg);
    background: var(--paper-raised);
    color: var(--ink);
}
.fill-dialog::backdrop {
    background: oklch(0.2 0.02 60 / 0.35);
}
.fill-dialog[open] {
    margin-top: 10vh;
}
.fill-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 18px;
    border-bottom: 1px solid var(--rule-subtle);
}
.fill-head h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 650;
}
.fill-head p {
    margin: 2px 0 0;
    font-size: 11.5px;
    color: var(--ink-tertiary);
}
.fill-body {
    padding: 16px 18px;
}
.fill-body .dseg {
    display: inline-flex;
    width: max-content;
    margin-bottom: 12px;
}
.fill-body .dseg button {
    flex: 0 0 auto;
}
.fill-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
}
.fill-grid .fld {
    flex: none;
    width: auto;
}
.fld-hint {
    font-size: 10px;
    font-style: normal;
    color: var(--ink-tertiary);
}
.fill-err {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    margin: 10px 0 0;
    font-size: 11.5px;
    color: var(--stamp-oxide);
}
.fill-err span {
    font-weight: 700;
}
.fill-sum {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 12px 0 0;
    padding-top: 11px;
    border-top: 1px dashed var(--rule-subtle);
    font-size: 11.5px;
    color: var(--ink-secondary);
}
.fill-sum b {
    font-family: var(--font-ledger);
    font-weight: 500;
    color: var(--ink);
}
.fill-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--rule-subtle);
    background: var(--paper);
}
.fill-foot .hint {
    margin: 0 auto 0 0;
    max-width: 34ch;
    font-size: 10.5px;
}
@media (max-width: 520px) {
    .fill-grid {
        grid-template-columns: minmax(0, 1fr);
    }
    .fill-foot {
        flex-wrap: wrap;
    }
    .fill-foot .hint {
        flex-basis: 100%;
        max-width: none;
        margin: 0 0 4px;
    }
}
.fld {
    display: grid;
    gap: 3px;
    flex: 0 1 128px;
    min-width: 0;
}
.fld span {
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.fld input,
.fld select {
    height: 30px;
    min-height: 30px;
    padding: 0 8px;
    font-family: var(--font-ledger);
    font-size: 12.5px;
    border: 1px solid var(--rule-strong);
    border-radius: var(--radius-sm);
    background: var(--paper-raised);
    color: var(--ink);
    width: 100%;
}
.fld select {
    font-family: var(--font-form);
}
.hint.wide {
    flex-basis: 100%;
    margin: 0;
}
.danger[data-armed="true"] {
    border-color: var(--stamp-oxide);
    color: var(--stamp-oxide);
}
.panel-foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 9px 16px;
    border-top: 1px solid var(--rule-subtle);
    background: var(--paper);
}
.empty-note {
    padding: 16px;
    margin: 0;
    font-size: 12px;
    color: var(--ink-tertiary);
    text-align: center;
}

/* archive and delete */
.manage {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 14px 16px;
}
.manage .hint {
    max-width: 74ch;
    margin: 0;
}
.manage-acts {
    display: flex;
    gap: 6px;
    flex: none;
}
:deep(.delete-instrument) {
    display: flex;
}

/* the phone's own action bar */
.actbar {
    display: none;
}
.only-mobile {
    display: none;
}
.rows .row {
    display: grid;
    grid-template-columns: 54px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 9px 14px;
    border-top: 1px solid var(--rule-subtle);
    font-size: 11.5px;
}
.rows .row:first-child {
    border-top: 0;
}
.rows .who {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    overflow: hidden;
}
.rows .when {
    color: var(--ink-tertiary);
}
.rows .sym {
    color: var(--ink-tertiary);
    font-size: 10.5px;
}

@media (max-width: 900px) {
    .pg-eyebrow {
        display: none;
    }
    .pg-title {
        font-size: 26px;
        margin: 0;
    }
    .price b {
        font-size: 30px;
    }
    .pg-top {
        flex-direction: column;
        align-items: stretch;
        gap: 6px;
    }
    .pg-side {
        justify-items: start;
        gap: 8px;
    }
    .price {
        justify-content: flex-start;
    }
    .pg-acts {
        display: none;
    }
    .hero :deep(.spark) {
        padding: 4px 16px 0;
    }
    .only-desk {
        display: none;
    }
    .only-mobile {
        display: block;
    }
    .band {
        grid-template-columns: minmax(0, 1fr);
        gap: 14px;
        margin-bottom: 10px;
    }
    .band > div {
        padding: 12px 14px;
    }
    .band h3 {
        margin-bottom: 6px;
    }
    .kv2 {
        border-bottom: 1px dotted var(--rule-subtle);
        padding: 5px 0;
    }
    .kv2:last-child {
        border-bottom: 0;
    }
    .kv2.split {
        margin-top: 0;
        border-top: 0;
        padding-top: 5px;
    }
    .fills .c-when {
        width: 104px;
    }
    .fills .c-side {
        width: 64px;
    }
    .fills .c-qty,
    .fills .c-price {
        width: 74px;
    }
    .fills .c-value {
        width: 92px;
    }
    .fills .c-act {
        width: 48px;
    }
    .dec .c-when {
        width: 104px;
    }
    .dec .c-status {
        width: 78px;
    }
    .dec .c-side {
        width: 62px;
    }
    .manage {
        flex-direction: column;
        align-items: stretch;
        gap: 10px;
    }
    .manage-acts {
        display: flex;
        gap: 6px;
    }
    .pg {
        padding-bottom: 64px;
    }
    .actbar {
        position: fixed;
        left: 0;
        right: 0;
        bottom: 54px;
        display: flex;
        gap: 6px;
        padding: 8px 16px;
        background: var(--paper-raised);
        border-top: 1px solid var(--rule);
        z-index: 15;
    }
    .actbar .d-btn {
        min-height: 38px;
        font-size: 12px;
    }
    .actbar .grow {
        flex: 1;
        justify-content: center;
    }
}
</style>
