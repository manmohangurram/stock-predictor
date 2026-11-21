<script setup lang="ts">
/**
 * What left the book, and what its fills actually made.
 *
 * There is no price left to value an archived instrument at, so the profit comes from the fills
 * themselves — realised on the sells, and money in minus money out for the round trip. The page
 * lists only instruments a person archived; a name the scan merely noticed is not here.
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Archive, ChevronRight } from "lucide-vue-next";
import { api, type ArchivedInstrument } from "../api";
import { histDate, money } from "../format";
import AppEmpty from "../components/ui/AppEmpty.vue";

const router = useRouter();
const rows = ref<ArchivedInstrument[]>([]);
const error = ref("");

onMounted(async () => {
    try {
        rows.value = await api.archived();
    } catch (e) {
        error.value = String(e);
    }
});

const total = computed(() => rows.value.reduce((n, r) => n + r.realised, 0));
const fills = computed(() => rows.value.reduce((n, r) => n + r.fills, 0));

const stocks = computed(() => rows.value.filter((r) => r.class === "stock"));
const funds = computed(() => rows.value.filter((r) => r.class !== "stock"));

type Filter = "all" | "stock" | "fund";
const filter = ref<Filter>("all");
const visible = computed(() =>
    filter.value === "all"
        ? rows.value
        : filter.value === "stock"
          ? stocks.value
          : funds.value,
);
const empty = computed(() =>
    filter.value === "all"
        ? "Nothing archived yet. Archive an instrument from its page — its record lands here."
        : filter.value === "stock"
          ? "No stocks archived."
          : "No funds archived.",
);

const signed = (n: number): string =>
    `${n > 0.0005 ? "+" : n < -0.0005 ? "−" : ""}${money(Math.abs(n))}`;

const totalTone = computed(() =>
    total.value > 0.0005 ? "up" : total.value < -0.0005 ? "down" : "",
);
const totalText = computed(() => signed(total.value));

/** An open round trip has nothing realised yet: an em dash, not a confident zero. */
function realised(r: ArchivedInstrument): { text: string; tone: string } {
    const nothing = Math.abs(r.realised) < 0.0005 && r.net_invested > 0.0005;
    if (r.fills === 0 || nothing) return { text: "—", tone: "" };
    return {
        text: signed(r.realised),
        tone: r.realised > 0.0005 ? "up" : r.realised < -0.0005 ? "down" : "",
    };
}

/** The phone row says how the round trip ended, not a price it no longer has. */
const phoneTail = (r: ArchivedInstrument): string =>
    Math.abs(r.net_invested) > 0.0005
        ? r.last_fill
            ? histDate(r.last_fill)
            : "—"
        : "closed out";

const visit = (symbol: string): void =>
    void router.push(`/i/${encodeURIComponent(symbol)}`);
</script>

<template>
    <div v-if="error" class="error-band">Something failed: {{ error }}</div>
    <template v-else>
        <header class="pg-top">
            <div class="pg-main">
                <div class="pg-eyebrow">Closed positions</div>
                <h1 class="pg-title">Archived</h1>
                <div class="pg-sub desk-only">
                    What left the book, and what its fills actually made
                </div>
                <div class="pg-sub only-mobile">
                    {{ rows.length }}
                    {{ rows.length === 1 ? "instrument" : "instruments" }}
                    · realised {{ totalText }}
                </div>
            </div>
            <div class="pg-side">
                <div class="pg-kpis">
                    <div>
                        <span>Instruments</span
                        ><b class="num">{{ rows.length }}</b>
                    </div>
                    <div>
                        <span>Fills kept</span><b class="num">{{ fills }}</b>
                    </div>
                    <div>
                        <span>Realised</span
                        ><b class="num" :class="totalTone">{{ totalText }}</b>
                    </div>
                </div>
            </div>
        </header>

        <section class="panel">
            <div class="panel-head">
                <h2>
                    <span class="desk-only">Closed positions</span>
                    <span class="only-mobile">Closed</span>
                </h2>
                <div
                    class="mseg"
                    role="group"
                    aria-label="Filter archived positions"
                >
                    <button
                        type="button"
                        :class="{ on: filter === 'all' }"
                        :aria-pressed="filter === 'all'"
                        @click="filter = 'all'"
                    >
                        All<span class="count desk-only">{{
                            rows.length
                        }}</span>
                    </button>
                    <button
                        type="button"
                        :class="{ on: filter === 'stock' }"
                        :aria-pressed="filter === 'stock'"
                        @click="filter = 'stock'"
                    >
                        Stocks<span class="count desk-only">{{
                            stocks.length
                        }}</span>
                    </button>
                    <button
                        type="button"
                        :class="{ on: filter === 'fund' }"
                        :aria-pressed="filter === 'fund'"
                        @click="filter = 'fund'"
                    >
                        Funds<span class="count desk-only">{{
                            funds.length
                        }}</span>
                    </button>
                </div>
            </div>

            <table class="arch-table only-desk">
                <colgroup>
                    <col style="width: 29%" />
                    <col style="width: 96px" />
                    <col style="width: 76px" />
                    <col style="width: 100px" />
                    <col style="width: 116px" />
                    <col style="width: 140px" />
                    <col style="width: 34px" />
                </colgroup>
                <thead>
                    <tr>
                        <th scope="col">Holding</th>
                        <th scope="col">Class</th>
                        <th scope="col" class="r">Fills</th>
                        <th scope="col" class="r">Decisions</th>
                        <th scope="col" class="r">Last fill</th>
                        <th scope="col" class="r">Realised</th>
                        <th scope="col"><span class="sr-only">Open</span></th>
                    </tr>
                </thead>
                <tbody>
                    <tr
                        v-for="r in visible"
                        :key="r.id"
                        tabindex="0"
                        role="link"
                        :aria-label="`${r.name}, archived — open its record`"
                        @click="visit(r.symbol)"
                        @keydown.enter="visit(r.symbol)"
                    >
                        <td class="hold">
                            <b>{{ r.name }}</b><span class="sym">{{
                                r.symbol
                            }}</span>
                        </td>
                        <td>
                            <span
                                class="tag"
                                :class="{ fund: r.class !== 'stock' }"
                                >{{
                                    r.class === "stock" ? "STOCK" : "FUND"
                                }}</span
                            >
                        </td>
                        <td class="r num">{{ r.fills }}</td>
                        <td class="r num">{{ r.decisions }}</td>
                        <td class="r num">
                            {{ r.last_fill ? histDate(r.last_fill) : "—" }}
                        </td>
                        <td class="r num" :class="realised(r).tone">
                            {{ realised(r).text }}
                        </td>
                        <td class="r">
                            <ChevronRight
                                class="chev"
                                :size="14"
                                :stroke-width="1.75"
                                aria-hidden="true"
                            />
                        </td>
                    </tr>
                    <tr v-if="!visible.length">
                        <td colspan="7">
                            <AppEmpty compact :icon="Archive" :text="empty" />
                        </td>
                    </tr>
                </tbody>
            </table>

            <div class="arch-rows only-mobile">
                <div
                    v-for="r in visible"
                    :key="r.id"
                    class="arch-row"
                    tabindex="0"
                    role="link"
                    :aria-label="`${r.name}, archived — open its record`"
                    @click="visit(r.symbol)"
                    @keydown.enter="visit(r.symbol)"
                >
                    <div>
                        <div class="nm">{{ r.name }}</div>
                        <div class="sy">
                            {{ r.symbol }} · {{ r.fills }}
                            {{ r.fills === 1 ? "fill" : "fills" }} ·
                            {{ phoneTail(r) }}
                        </div>
                    </div>
                    <div class="val">
                        <div class="num" :class="realised(r).tone">
                            {{ realised(r).text }}
                        </div>
                        <div class="hint">realised</div>
                    </div>
                </div>
                <AppEmpty
                    v-if="!visible.length"
                    compact
                    :icon="Archive"
                    :text="empty"
                />
            </div>
        </section>
    </template>
</template>

<style scoped>
.desk-only {
    display: inline;
}
.only-mobile {
    display: none;
}
.count {
    margin-left: 4px;
}

.arch-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    table-layout: fixed;
}
.arch-table th {
    font-size: 9.5px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    text-align: left;
    padding: 8px 16px;
    background: var(--paper-soft);
    font-weight: 700;
}
.arch-table th.r,
.arch-table td.r {
    text-align: right;
}
.arch-table td {
    padding: 10px 16px;
    border-top: 1px solid var(--rule-subtle);
    vertical-align: middle;
    overflow-wrap: anywhere;
}
.arch-table tbody tr {
    cursor: pointer;
}
.arch-table tbody tr:hover {
    background: var(--paper-soft);
}
.arch-table tbody tr:focus-visible {
    outline: 2px solid var(--rule-focus);
    outline-offset: -2px;
}
.arch-table .hold b {
    font-weight: 700;
}
.sym {
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--ink-tertiary);
    margin-left: 6px;
}
.chev {
    color: var(--ink-tertiary);
    vertical-align: middle;
}

@media (max-width: 900px) {
    .desk-only {
        display: none;
    }
    .only-desk {
        display: none;
    }
    .only-mobile {
        display: block;
    }
    .pg-top {
        flex-direction: column;
        align-items: stretch;
        gap: 0;
        margin-bottom: 12px;
    }
    .pg-side,
    .pg-eyebrow {
        display: none;
    }
    .pg-title {
        font-size: 27px;
    }
    .pg-sub {
        font-size: 11px;
        margin-top: 2px;
    }
    .panel-head {
        padding: 12px 14px 6px;
        border-bottom: 0;
    }
    .panel-head h2 {
        font-size: 9px;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--ink-tertiary);
    }
    .arch-row {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 10px;
        padding: 10px 14px;
        border-bottom: 1px solid var(--rule-subtle);
        background: var(--paper-raised);
        cursor: pointer;
    }
    .arch-row:last-of-type {
        border-bottom: 0;
    }
    .arch-row:focus-visible {
        outline: 2px solid var(--rule-focus);
        outline-offset: -2px;
    }
    .arch-row .nm {
        font-weight: 650;
        font-size: 12.5px;
    }
    .arch-row .sy {
        font-family: var(--font-ledger);
        font-size: 10px;
        color: var(--ink-tertiary);
    }
    .arch-row .val {
        text-align: right;
    }
    .arch-row .hint {
        font-size: 10.5px;
        color: var(--ink-tertiary);
    }
}
</style>
