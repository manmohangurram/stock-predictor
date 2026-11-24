<script setup lang="ts">
/**
 * The audit: every outbound model call, newest first, with the schedule above it and the stored
 * exchange under any row.
 *
 * Filters and paging are server-side and live in the URL — the list is the record, and a link to
 * page 3 of one model's calls has to work. The auto-refresh tick is the reader's choice, remembered
 * across reloads: this page is a live log while a run is happening and a still record otherwise.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ChevronRight, Inbox } from "lucide-vue-next";
import { api, type CallRow, type RunRow, type ScheduledPrompt } from "../api";
import { clock, dayTime, humanLabel, titleCase } from "../format";
import AppEmpty from "../components/ui/AppEmpty.vue";
import FilterMenu from "../components/ui/FilterMenu.vue";
import ScheduleStrip from "../components/ScheduleStrip.vue";
import CallDrawer from "../components/CallDrawer.vue";

interface Filters {
    prompt?: string;
    channel?: string;
    model?: string;
    review?: string;
}

const route = useRoute();
const router = useRouter();

const prompts = ref<ScheduledPrompt[]>([]);
const now = ref(new Date().toISOString());
const rows = ref<CallRow[]>([]);
const loadedAt = ref<Date | null>(null);
const runs = ref<RunRow[]>([]);
const facets = ref<{ prompts: string[]; calls: string[]; models: string[] }>({
    prompts: [],
    calls: [],
    models: [],
});
const total = ref(0);
const running = ref(0);
const error = ref("");
const open = ref<string | null>(null);
const INTERVALS = [5, 15, 30, 60] as const;
const every = ref(5);
const auto = ref(true);
let timer: number | undefined;

const limit = computed(() => {
    const n = Number(route.query.limit);
    return [25, 50, 100].includes(n) ? n : 25;
});
const offset = computed(() =>
    Math.max(0, Number(route.query.offset ?? 0) || 0),
);
const filters = computed<Filters>(() => ({
    prompt: (route.query.prompt as string) || undefined,
    channel: (route.query.channel as string) || undefined,
    model: (route.query.model as string) || undefined,
    review: (route.query.review as string) || undefined,
}));
const filtered = computed(() => Object.values(filters.value).some(Boolean));

const tokens = computed(() =>
    rows.value.reduce(
        (n, r) => n + (r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0),
        0,
    ),
);
const openReview = computed(
    () => rows.value.filter((r) => r.decision_status === "pending").length,
);
const runsToday = computed(() => {
    const today = new Date().toDateString();
    return runs.value.filter(
        (r) => new Date(r.started_at).toDateString() === today,
    ).length;
});

const tokenText = (n: number): string =>
    n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);

const rangeText = computed(() => {
    if (total.value === 0) return "0 of 0";
    const end = Math.min(offset.value + limit.value, total.value);
    return `Showing ${offset.value + 1}–${end} of ${total.value}${
        filtered.value ? " · filtered" : " · newest first"
    }`;
});

async function load(): Promise<void> {
    try {
        const [schedule, f, list, log] = await Promise.all([
            api.schedule(),
            api.callFacets(),
            api.calls(offset.value, limit.value, filters.value),
            api.runs(50),
        ]);
        prompts.value = schedule.prompts;
        now.value = schedule.now;
        facets.value = f;
        rows.value = list.rows;
        total.value = list.total;
        runs.value = log.runs;
        running.value = log.running;
        loadedAt.value = new Date();
        error.value = "";
    } catch (e) {
        error.value = String(e);
    }
}

/** Set one filter in the URL, from the top: page 2 of the old filter is not page 2 of the new one. */
function setFilter(name: keyof Filters, value: string): void {
    const q = { ...route.query, offset: undefined } as Record<
        string,
        string | undefined
    >;
    if (value) q[name] = value;
    else delete q[name];
    void router.push({ path: "/calls", query: q });
}

function clearFilters(): void {
    void router.push({ path: "/calls" });
}

function page(next: number): void {
    void router.push({
        path: "/calls",
        query: { ...route.query, offset: String(next) },
    });
}

function pageSize(size: string): void {
    void router.push({
        path: "/calls",
        query: { offset: "0", limit: size, ...filters.value },
    });
}

const toggle = (id: string): void => {
    open.value = open.value === id ? null : id;
};

const promptType = (r: CallRow): string => {
    if (r.endpoint === "search") return "Web search";
    if (r.endpoint === "fetch") return "Web fetch";
    const [kind] = (r.purpose ?? "").split(/\s+/);
    return titleCase(kind || "—");
};

/** The template that was sent, which is what the Prompt column names. */
const promptName = (r: CallRow): string => r.model ?? promptType(r);

/** One line that says what came back, without opening the exchange. */
function resultText(r: CallRow): string {
    if (r.error) return "failed";
    const legs = r.plan_legs ?? 0;
    if (legs > 0) return `${legs} ${legs === 1 ? "move" : "moves"}`;
    if (r.decision_action === "BLOCKED") return "blocked";
    if (r.decision_status) {
        return r.instrument_name
            ? `${r.decision_status} · ${r.instrument_name}`
            : r.decision_status;
    }
    return r.instrument_name ?? "no action";
}

const resultTone = (r: CallRow): string =>
    r.error || r.decision_action === "BLOCKED" ? "down" : "";

const rowTokens = (r: CallRow): string =>
    tokenText((r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0));

/** The tick and the interval are the reader's, so they outlive a render. */
function arm(): void {
    window.clearInterval(timer);
    if (!auto.value) return;
    timer = window.setInterval(() => {
        if (document.hidden) return;
        const el = document.activeElement;
        if (
            el instanceof HTMLInputElement ||
            el instanceof HTMLSelectElement ||
            el instanceof HTMLTextAreaElement
        ) {
            return;
        }
        void load();
    }, every.value * 1000);
}

function remember(): void {
    window.localStorage.setItem("calls.autoRefresh", auto.value ? "on" : "off");
    window.localStorage.setItem("calls.autoEvery", String(every.value));
    arm();
}

function onVisible(): void {
    if (!document.hidden) void load();
}

onMounted(async () => {
    auto.value = window.localStorage.getItem("calls.autoRefresh") !== "off";
    const stored = Number(window.localStorage.getItem("calls.autoEvery"));
    every.value = (INTERVALS as readonly number[]).includes(stored)
        ? stored
        : 5;
    arm();
    document.addEventListener("visibilitychange", onVisible);
    await load();
});
onBeforeUnmount(() => {
    window.clearInterval(timer);
    document.removeEventListener("visibilitychange", onVisible);
});
watch(() => route.fullPath, load);
</script>

<template>
    <div v-if="error" class="error-band">
        Something failed: {{ error }}
        <button class="d-btn d-btn-sm" @click="load()">Retry</button>
    </div>
    <template v-else>
        <header class="pg-top">
            <div class="pg-main">
                <div class="pg-eyebrow">Scheduler · model calls</div>
                <h1 class="pg-title">Audit</h1>
                <div class="pg-sub desk-only">
                    What ran, when it runs next, and what each call answered
                </div>
                <div class="pg-sub only-mobile">
                    {{ runsToday }} runs · {{ total }} calls ·
                    {{ openReview }} open
                </div>
            </div>
            <div class="pg-side">
                <div class="pg-kpis">
                    <div>
                        <span>Runs today</span
                        ><b class="num">{{ runsToday }}</b>
                    </div>
                    <div>
                        <span>Calls</span><b class="num">{{ total }}</b>
                    </div>
                    <div>
                        <span>Tokens</span
                        ><b class="num">{{ tokenText(tokens) }}</b>
                    </div>
                    <div>
                        <span>Open review</span
                        ><b class="num">{{ openReview }}</b>
                    </div>
                </div>
            </div>
        </header>

        <ScheduleStrip
            :prompts="prompts"
            :now="now"
            :running="running"
            @reload="load()"
        />

        <section class="panel">
            <div class="panel-head">
                <h2>Calls</h2>
                <span class="r"
                    >{{ rangeText }} · updated
                    {{ loadedAt ? clock(loadedAt.toISOString()) : "—" }}</span
                >
            </div>

            <div class="filters">
                <FilterMenu
                    label="Prompt"
                    :options="facets.prompts"
                    :model-value="filters.prompt"
                    humanise
                    @update:model-value="setFilter('prompt', $event)"
                />
                <FilterMenu
                    label="Call"
                    :options="facets.calls"
                    :model-value="filters.channel"
                    humanise
                    @update:model-value="setFilter('channel', $event)"
                />
                <FilterMenu
                    label="Model"
                    :options="facets.models"
                    :model-value="filters.model"
                    humanise
                    @update:model-value="setFilter('model', $event)"
                />
                <FilterMenu
                    label="Review"
                    :options="[
                        'pending',
                        'approved',
                        'rejected',
                        'blocked',
                        'none',
                    ]"
                    :model-value="filters.review"
                    @update:model-value="setFilter('review', $event)"
                />
                <button
                    v-if="filtered"
                    class="d-btn d-btn-sm d-btn-ghost"
                    type="button"
                    @click="clearFilters()"
                >
                    Clear
                </button>
                <div class="tick">
                    <label
                        class="tick-toggle"
                        title="Reload this list on a timer"
                    >
                        <input
                            v-model="auto"
                            type="checkbox"
                            @change="remember"
                        />
                        <span class="tick-dot" aria-hidden="true"></span>
                        <span class="tick-word">Auto-refresh</span>
                    </label>
                    <span class="tick-sep" aria-hidden="true">·</span>
                    <select
                        v-model.number="every"
                        aria-label="Refresh interval"
                        @change="remember"
                    >
                        <option v-for="s in INTERVALS" :key="s" :value="s">
                            {{ s === 60 ? "1 min" : `${s}s` }}
                        </option>
                    </select>
                </div>
            </div>

            <div class="call-list only-desk">
                <div class="shead">
                    <span>Time</span>
                    <span>Prompt</span>
                    <span>Call</span>
                    <span>Result</span>
                    <span class="r">Tokens</span>
                    <span><span class="sr-only">Open</span></span>
                </div>
                <div
                    v-for="r in rows"
                    :key="r.id"
                    class="audit-call"
                    :class="{ open: open === r.id }"
                >
                    <div
                        class="srow"
                        role="button"
                        tabindex="0"
                        :aria-expanded="open === r.id"
                        :aria-label="`${promptType(r)} call at ${dayTime(r.ts)} — ${
                            open === r.id ? 'hide' : 'show'
                        } the exchange`"
                        @click="toggle(r.id)"
                        @keydown.enter.prevent="toggle(r.id)"
                        @keydown.space.prevent="toggle(r.id)"
                    >
                        <span class="num when">{{ dayTime(r.ts) }}</span>
                        <span class="mono prompt">{{ promptName(r) }}</span>
                        <span class="sym">{{
                            titleCase(r.endpoint)
                        }}</span>
                        <span class="hint res" :class="resultTone(r)">{{
                            resultText(r)
                        }}</span>
                        <span class="toks">{{ rowTokens(r) }}</span>
                        <ChevronRight
                            class="chev"
                            :class="{ open: open === r.id }"
                            :size="13"
                            :stroke-width="1.75"
                            aria-hidden="true"
                        />
                    </div>
                    <div v-if="open === r.id" class="sdrawer">
                        <CallDrawer :id="r.id" />
                    </div>
                </div>
            </div>

            <div class="calls-m only-mobile">
                <div
                    v-for="r in rows"
                    :key="r.id"
                    class="audit-call"
                    :class="{ open: open === r.id }"
                >
                    <div
                        class="prow"
                        role="button"
                        tabindex="0"
                        :aria-expanded="open === r.id"
                        :aria-label="`${promptType(r)} call at ${dayTime(r.ts)} — ${
                            open === r.id ? 'hide' : 'show'
                        } the exchange`"
                        @click="toggle(r.id)"
                        @keydown.enter.prevent="toggle(r.id)"
                        @keydown.space.prevent="toggle(r.id)"
                    >
                        <div>
                            <div class="nm mono">{{ promptName(r) }}</div>
                            <div class="sy">
                                {{ titleCase(r.endpoint) }} ·
                                {{ dayTime(r.ts) }}
                            </div>
                        </div>
                        <div class="right">
                            <div class="num">{{ rowTokens(r) }}</div>
                            <div class="hint" :class="resultTone(r)">
                                {{ resultText(r) }}
                            </div>
                        </div>
                    </div>
                    <div v-if="open === r.id" class="pdrawer">
                        <CallDrawer :id="r.id" compact />
                    </div>
                </div>
            </div>

            <AppEmpty
                v-if="!rows.length"
                :icon="Inbox"
                :text="
                    filtered
                        ? 'No calls match these filters.'
                        : 'No calls yet. The next scheduled run writes its exchange here.'
                "
            >
                <button
                    v-if="filtered"
                    class="d-btn d-btn-sm"
                    type="button"
                    @click="clearFilters()"
                >
                    Clear filters
                </button>
            </AppEmpty>

            <div class="pager">
                <span>{{ rangeText }}</span>
                <span class="pg-right">
                    <label class="pg-size">
                        Show
                        <select
                            :value="String(limit)"
                            aria-label="Calls per page"
                            @change="
                                pageSize(
                                    ($event.target as HTMLSelectElement).value,
                                )
                            "
                        >
                            <option
                                v-for="n in [25, 50, 100]"
                                :key="n"
                                :value="n"
                            >
                                {{ n }} per page
                            </option>
                        </select>
                    </label>
                    <button
                        class="d-btn d-btn-sm"
                        type="button"
                        :disabled="offset === 0"
                        @click="page(Math.max(0, offset - limit))"
                    >
                        Previous
                    </button>
                    <button
                        class="d-btn d-btn-sm"
                        type="button"
                        :disabled="offset + limit >= total"
                        @click="page(offset + limit)"
                    >
                        Next
                    </button>
                </span>
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
.only-desk {
    display: block;
}

.filters {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
    padding: 10px 14px;
    border-bottom: 1px solid var(--rule-subtle);
}
.filters .tick {
    margin-left: auto;
}

.call-list {
    min-width: 0;
}
.shead,
.srow {
    display: grid;
    grid-template-columns: 104px 132px 108px minmax(0, 1fr) 90px 22px;
    gap: 12px;
    align-items: center;
}
.shead {
    padding: 7px 16px;
    background: var(--paper-soft);
    font-size: 9px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.shead .r {
    text-align: right;
}
.audit-call {
    border-top: 1px solid var(--rule-subtle);
}
.audit-call.open {
    background: var(--paper-soft);
}
.srow {
    padding: 10px 16px;
    font-size: 12px;
    cursor: pointer;
}
.srow:hover {
    background: var(--paper-soft);
}
.srow:focus-visible {
    outline: 2px solid var(--rule-focus);
    outline-offset: -2px;
}
.srow .when {
    font-size: 11.5px;
    color: var(--ink-secondary);
}
.srow .mono {
    font-family: var(--font-ledger);
    font-size: 11.5px;
}
.srow .sym {
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--ink-tertiary);
}
.srow .hint {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--ink-tertiary);
}
.srow .hint.down {
    color: var(--stamp-oxide);
}
.toks {
    font-family: var(--font-ledger);
    font-size: 10px;
    color: var(--ink-tertiary);
    text-align: right;
}
.chev {
    color: var(--ink-tertiary);
    transition: transform 120ms ease-out;
}
.chev.open {
    transform: rotate(90deg);
}
.sdrawer {
    padding: 0 16px 12px;
}

.pager {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 16px;
    border-top: 1px solid var(--rule-subtle);
    background: var(--paper-soft);
    font-size: 11px;
    color: var(--ink-tertiary);
}
.pg-right {
    display: flex;
    align-items: center;
    gap: 6px;
}
.pg-size {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-right: 4px;
}
.pg-size select {
    min-height: 28px;
    padding: 2px 22px 2px 8px;
    font-family: var(--font-ledger);
    font-size: 11px;
    background-size: 10px 10px;
    background-position: right 6px center;
}

@media (max-width: 900px) {
    .desk-only {
        display: none;
    }
    .only-mobile {
        display: block;
    }
    .only-desk {
        display: none;
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
        padding: 11px 14px 6px;
        border-bottom: 0;
    }
    .panel-head h2 {
        font-size: 9px;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--ink-tertiary);
    }
    .panel-head .r {
        font-size: 9px;
        letter-spacing: 0.12em;
        text-transform: uppercase;
    }
    .filters {
        padding: 10px 14px;
        gap: 6px;
    }
    .filters .tick {
        margin-left: 0;
    }
    .prow {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 10px;
        align-items: center;
        padding: 10px 14px;
        cursor: pointer;
    }
    .prow:focus-visible {
        outline: 2px solid var(--rule-focus);
        outline-offset: -2px;
    }
    .prow .nm {
        font-size: 11.5px;
    }
    .prow .sy {
        font-family: var(--font-ledger);
        font-size: 10px;
        color: var(--ink-tertiary);
    }
    .prow .right {
        text-align: right;
    }
    .prow .hint {
        font-size: 10.5px;
        color: var(--ink-tertiary);
    }
    .prow .hint.down {
        color: var(--stamp-oxide);
    }
    .pdrawer {
        margin: 0 12px 10px;
        padding: 12px;
        background: var(--paper-raised);
        border: 1px solid var(--rule-subtle);
        border-radius: var(--radius-card);
        box-shadow: var(--shadow-card);
    }
    .pager {
        padding: 10px 14px;
    }
}
</style>
