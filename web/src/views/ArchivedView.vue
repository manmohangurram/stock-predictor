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
import { api, type ArchivedInstrument } from "../api";
import { histDate, money } from "../format";

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

/** An open round trip has nothing realised yet: an em dash, not a confident zero. */
function realised(r: ArchivedInstrument): { text: string; tone: string } {
    const nothing = Math.abs(r.realised) < 0.0005 && r.net_invested > 0.0005;
    if (r.fills === 0 || nothing) return { text: "—", tone: "" };
    return {
        text: `${r.realised >= 0 ? "+" : "−"}${money(Math.abs(r.realised))}`,
        tone: r.realised > 0.0005 ? "up" : r.realised < -0.0005 ? "down" : "",
    };
}

const open = (r: ArchivedInstrument): string =>
    Math.abs(r.net_invested) > 0.0005
        ? `${money(Math.abs(r.net_invested))} ${r.net_invested > 0 ? "in" : "out"}`
        : "closed out";

const visit = (symbol: string): void =>
    void router.push(`/i/${encodeURIComponent(symbol)}`);
</script>

<template>
    <div v-if="error" class="error-band">Something failed: {{ error }}</div>
    <template v-else>
        <div class="page-head">
            <div>
                <h1>Archived</h1>
                <div class="page-sub">
                    What left the book, and what it made — history without the
                    prices
                </div>
            </div>
            <div class="head-right">
                <div class="head-row">
                    <div class="figures">
                        <div>
                            <span class="lbl">Instruments</span
                            ><span class="fig-line"
                                ><b class="num">{{ rows.length }}</b></span
                            >
                        </div>
                        <div>
                            <span class="lbl">Fills kept</span
                            ><span class="fig-line"
                                ><b class="num">{{ fills }}</b></span
                            >
                        </div>
                        <div>
                            <span class="lbl">Realised</span
                            ><span class="fig-line">
                                <b
                                    class="num"
                                    :class="
                                        total === 0
                                            ? ''
                                            : total > 0
                                              ? 'pos'
                                              : 'neg'
                                    "
                                    >{{
                                        total === 0
                                            ? money(0, 0)
                                            : `${total > 0 ? "+" : "−"}${money(Math.abs(total), 0)}`
                                    }}</b
                                ></span
                            >
                        </div>
                    </div>
                </div>
            </div>
        </div>

        <div
            class="arch-filter"
            role="group"
            aria-label="Filter archived positions"
        >
            <button
                type="button"
                :class="{ on: filter === 'all' }"
                :aria-pressed="filter === 'all'"
                @click="filter = 'all'"
            >
                All {{ rows.length }}
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

        <section class="sect">
            <div class="table-wrap">
                <table class="blotter">
                    <thead>
                        <tr>
                            <th scope="col">Holding</th>
                            <th scope="col" class="hide-sm num">Fills</th>
                            <th scope="col" class="hide-sm num">Decisions</th>
                            <th scope="col" class="hide-sm num">Last fill</th>
                            <th scope="col" class="num">Realised</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr
                            v-for="r in visible"
                            :key="r.id"
                            class="row"
                            tabindex="0"
                            role="link"
                            :aria-label="`${r.name}, archived — open its record`"
                            @click="visit(r.symbol)"
                            @keydown.enter="visit(r.symbol)"
                        >
                            <td class="holding">
                                <div class="name">{{ r.name }}</div>
                                <div class="sub">
                                    {{ r.symbol }} ·
                                    {{ r.class.replace("_", " ") }}
                                </div>
                            </td>
                            <td class="num hide-sm">{{ r.fills }}</td>
                            <td class="num hide-sm">{{ r.decisions }}</td>
                            <td class="num hide-sm">
                                {{ r.last_fill ? histDate(r.last_fill) : "—" }}
                            </td>
                            <td class="num" :class="realised(r).tone">
                                {{ realised(r).text }}
                                <div class="sub">{{ open(r) }}</div>
                            </td>
                        </tr>
                        <tr v-if="!visible.length">
                            <td colspan="5" class="empty">{{ empty }}</td>
                        </tr>
                    </tbody>
                </table>
            </div>
        </section>
    </template>
</template>

<style scoped>
.arch-filter {
  display: inline-flex;
  gap: 2px;
  background: var(--paper-inset);
  border-radius: 9px;
  padding: 3px;
  margin-bottom: var(--space-3);
}
.arch-filter button {
  border: 0;
  background: none;
  font: inherit;
  font-size: 12px;
  color: var(--ink-secondary);
  padding: 5px 11px;
  border-radius: 7px;
  cursor: pointer;
}
.arch-filter button.on {
  background: var(--paper-raised);
  color: var(--ink);
  font-weight: 650;
  box-shadow: 0 1px 2px rgb(16 23 21 / 0.08);
}
</style>
