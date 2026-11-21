<script setup lang="ts">
/**
 * The review queue: every decision in its own tab, beside the book it would change. Horizon and
 * status are filters on the same list — a plan is one `plan_id`, and every leg posts together.
 *
 * Both purses are fetched so "All" can show long and short decisions at once; each card quotes the
 * purse its own horizon would spend from.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api, type Dashboard, type Mode, type SignalRow } from "../api";
import { money } from "../format";
import { debitOf } from "../lib/decision";
import DecisionCard from "../components/DecisionCard.vue";

type Horizon = "all" | Mode;
type Status = "pending" | "approved" | "rejected";

interface Decision {
    key: string;
    legs: SignalRow[];
    mode: Mode;
    plan: boolean;
}

const data = ref<{ long: Dashboard | null; trade: Dashboard | null }>({
    long: null,
    trade: null,
});
const error = ref("");
const loaded = ref(false);
const horizon = ref<Horizon>("all");
const status = ref<Status>("pending");

const HORIZONS: { id: Horizon; label: string }[] = [
    { id: "all", label: "All" },
    { id: "long", label: "Long" },
    { id: "trade", label: "Short" },
];

/** A plan's legs share one `plan_id`; the newest leg decides where the plan sits in the queue. */
function group(rows: SignalRow[], mode: Mode): Decision[] {
    const sorted = [...rows].sort((a, b) => (a.id < b.id ? 1 : -1));
    const plans = new Map<string, SignalRow[]>();
    for (const s of sorted) {
        if (s.plan_id)
            plans.set(s.plan_id, [...(plans.get(s.plan_id) ?? []), s]);
    }
    const out: Decision[] = [];
    const seen = new Set<string>();
    for (const s of sorted) {
        if (s.plan_id && plans.has(s.plan_id)) {
            if (seen.has(s.plan_id)) continue;
            seen.add(s.plan_id);
            const legs = [...plans.get(s.plan_id)!].sort(
                (a, b) =>
                    (a.action === "SELL" ? -1 : 1) -
                    (b.action === "SELL" ? -1 : 1),
            );
            out.push({ key: s.plan_id, legs, mode, plan: true });
            continue;
        }
        out.push({ key: s.id, legs: [s], mode, plan: false });
    }
    return out;
}

const all = computed<Decision[]>(() => {
    const rows = [
        ...group(data.value.long?.signals ?? [], "long"),
        ...group(data.value.trade?.signals ?? [], "trade"),
    ];
    return rows.sort((a, b) => (a.legs[0].id < b.legs[0].id ? 1 : -1));
});

const inHorizon = computed(() =>
    horizon.value === "all"
        ? all.value
        : all.value.filter((d) => d.mode === horizon.value),
);

const counts = computed(() => {
    const c: Record<Status, number> = { pending: 0, approved: 0, rejected: 0 };
    for (const d of inHorizon.value) c[d.legs[0].status] += 1;
    return c;
});

const shown = computed(() =>
    inHorizon.value.filter((d) => d.legs[0].status === status.value),
);

const waiting = computed(() =>
    inHorizon.value
        .filter((d) => d.legs[0].status === "pending")
        .reduce((n, d) => n + debitOf(d.legs), 0),
);

const freeFor = (mode: Mode): number => data.value[mode]?.available ?? 0;

const FILTERS: { id: Status; label: string }[] = [
    { id: "pending", label: "Pending" },
    { id: "approved", label: "Approved" },
    { id: "rejected", label: "Rejected" },
];

const sub = computed(() => {
    if (!loaded.value) return "Reading the queue…";
    const n = counts.value.pending;
    return n === 0
        ? "Nothing is waiting on you"
        : `${n} waiting · ${money(waiting.value)} needed`;
});

const emptyText = computed(() =>
    status.value === "pending"
        ? "Nothing under review. The next run writes its plan here."
        : status.value === "approved"
          ? "Nothing approved yet."
          : "Nothing rejected yet.",
);

async function load(): Promise<void> {
    try {
        const [long, trade] = await Promise.all([
            api.dashboard("long", "all"),
            api.dashboard("trade", "all"),
        ]);
        data.value = { long, trade };
        error.value = "";
    } catch (e) {
        error.value = String(e);
    } finally {
        loaded.value = true;
    }
}

let timer: number | undefined;
onMounted(async () => {
    await load();
    timer = window.setInterval(() => void load(), 60_000);
});
onBeforeUnmount(() => window.clearInterval(timer));
</script>

<template>
    <div class="pg">
        <div v-if="error" class="error-band">
            Something failed: {{ error }}
            <button class="d-btn d-btn-xs" type="button" @click="load()">
                Retry
            </button>
        </div>

        <header class="pg-top">
            <div>
                <div class="pg-eyebrow">Review queue</div>
                <h1 class="pg-title">Decisions</h1>
                <div class="pg-sub">{{ sub }}</div>
            </div>
            <div class="pg-side">
                <div class="dseg" role="group" aria-label="Horizon">
                    <button
                        v-for="h in HORIZONS"
                        :key="h.id"
                        type="button"
                        :class="{ on: horizon === h.id }"
                        :aria-pressed="horizon === h.id"
                        @click="horizon = h.id"
                    >
                        {{ h.label }}
                    </button>
                </div>
            </div>
        </header>

        <div class="filters">
            <div class="dseg" role="group" aria-label="Status">
                <button
                    v-for="f in FILTERS"
                    :key="f.id"
                    type="button"
                    :class="{ on: status === f.id }"
                    :aria-pressed="status === f.id"
                    @click="status = f.id"
                >
                    {{ f.label }} {{ counts[f.id] }}
                </button>
            </div>
            <span class="filters-note"
                >A plan is one decision; every leg posts together</span
            >
        </div>

        <DecisionCard
            v-for="d in shown"
            :key="d.key"
            :legs="d.legs"
            :plan="d.plan"
            :mode="d.mode"
            :free="freeFor(d.mode)"
            @reload="load()"
        />

        <p v-if="loaded && !shown.length" class="panel empty-note">
            {{ emptyText }}
        </p>
    </div>
</template>

<style scoped>
.filters {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    border: 1px solid var(--rule-subtle);
    border-radius: var(--radius-card);
    background: var(--paper-raised);
    padding: 8px 12px;
    margin-bottom: 14px;
}
.filters-note {
    margin-left: auto;
    font-size: 11px;
    color: var(--ink-tertiary);
}
.empty-note {
    padding: 18px 14px;
    margin: 0;
    text-align: center;
    font-size: 12px;
    color: var(--ink-tertiary);
    box-shadow: none;
}
@media (max-width: 640px) {
    .filters {
        padding: 8px 10px;
    }
    .filters .dseg {
        display: flex;
        width: 100%;
    }
    .filters .dseg button {
        flex: 1;
        text-align: center;
        padding: 6px 8px;
        font-size: 11px;
    }
    .filters-note {
        display: none;
    }
}
</style>
