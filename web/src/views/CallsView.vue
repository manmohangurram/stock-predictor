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
import { api, type CallRow, type ScheduledPrompt } from "../api";
import { dayTime, escapeHtml, humanLabel, titleCase } from "../format";
import { pill } from "../components/stamp";
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
const facets = ref<{ prompts: string[]; calls: string[]; models: string[] }>({
  prompts: [],
  calls: [],
  models: [],
});
const total = ref(0);
const running = ref(0);
const error = ref("");
const open = ref<number | null>(null);
const INTERVALS = [5, 15, 30, 60] as const;
const every = ref(5);
const auto = ref(true);
let timer: number | undefined;

const limit = computed(() => {
  const n = Number(route.query.limit);
  return [25, 50, 100].includes(n) ? n : 25;
});
const offset = computed(() => Math.max(0, Number(route.query.offset ?? 0) || 0));
const filters = computed<Filters>(() => ({
  prompt: (route.query.prompt as string) || undefined,
  channel: (route.query.channel as string) || undefined,
  model: (route.query.model as string) || undefined,
  review: (route.query.review as string) || undefined,
}));
const filtered = computed(() => Object.values(filters.value).some(Boolean));

const failures = computed(() => rows.value.filter((r) => r.error).length);
const tokens = computed(() =>
  rows.value.reduce((n, r) => n + (r.prompt_tokens ?? 0) + (r.completion_tokens ?? 0), 0),
);

async function load(): Promise<void> {
  try {
    const [schedule, f, list, runs] = await Promise.all([
      api.schedule(),
      api.callFacets(),
      api.calls(offset.value, limit.value, filters.value),
      api.runs(1),
    ]);
    prompts.value = schedule.prompts;
    now.value = schedule.now;
    facets.value = f;
    rows.value = list.rows;
    total.value = list.total;
    running.value = runs.running;
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

/** Set one filter in the URL, from the top: page 2 of the old filter is not page 2 of the new one. */
function setFilter(name: keyof Filters, value: string): void {
  const q = { ...route.query, offset: undefined } as Record<string, string | undefined>;
  if (value) q[name] = value;
  else delete q[name];
  void router.push({ path: "/calls", query: q });
}

function page(next: number): void {
  void router.push({ path: "/calls", query: { ...route.query, offset: String(next) } });
}

function pageSize(size: string): void {
  void router.push({ path: "/calls", query: { offset: "0", limit: size, ...filters.value } });
}

const promptType = (r: CallRow): string => {
  if (r.endpoint === "search") return "Web search";
  if (r.endpoint === "fetch") return "Web fetch";
  const [kind] = (r.purpose ?? "").split(/\s+/);
  return titleCase(kind || "—");
};

const holding = (r: CallRow): string => {
  const tool = r.endpoint === "search" || r.endpoint === "fetch";
  const [kind, ...rest] = (r.purpose ?? "").split(/\s+/);
  if (!tool && kind === "plan") return "Whole book";
  if (r.instrument_name) return r.instrument_name;
  return tool ? (r.purpose ?? "—") : rest.join(" ") || "—";
};
const holdingSub = (r: CallRow): string | null => {
  const [kind] = (r.purpose ?? "").split(/\s+/);
  if (kind === "plan") return "every holding";
  return r.instrument_symbol ?? null;
};

const reviewBadge = (r: CallRow): { tone: Parameters<typeof pill>[0]; label: string; title: string } => {
  if (r.error) return { tone: "failed", label: "failed", title: r.error };
  if (r.decision_action === "BLOCKED") {
    return {
      tone: "todo",
      label: (r.plan_legs ?? 0) > 0 ? "plan · refused" : "blocked",
      title: r.decision_reason ?? "",
    };
  }
  if (r.decision_action === "PLAN") {
    const state = r.decision_status ?? "pending";
    return {
      tone: state === "approved" ? "ok" : state === "rejected" ? "todo" : "",
      label: state === "pending" ? "plan · awaiting you" : `plan · ${state}`,
      title: "",
    };
  }
  if (!r.decision_status) return { tone: "", label: "", title: "" };
  return {
    tone: r.decision_status === "approved" ? "ok" : r.decision_status === "rejected" ? "todo" : "",
    label: r.decision_status === "pending" ? "awaiting you" : r.decision_status,
    title: r.decision_id ? `decision #${r.decision_id}` : "",
  };
};

/** The tick and the interval are the reader's, so they outlive a render. */
function arm(): void {
  window.clearInterval(timer);
  if (!auto.value) return;
  timer = window.setInterval(() => {
    if (document.hidden) return;
    const el = document.activeElement;
    if (el instanceof HTMLInputElement || el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement) {
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

onMounted(async () => {
  auto.value = window.localStorage.getItem("calls.autoRefresh") !== "off";
  const stored = Number(window.localStorage.getItem("calls.autoEvery"));
  every.value = (INTERVALS as readonly number[]).includes(stored) ? stored : 5;
  arm();
  await load();
});
onBeforeUnmount(() => window.clearInterval(timer));
watch(() => route.fullPath, load);
</script>

<template>
  <div v-if="error" class="error-band">Something failed: {{ error }}
    <button class="d-btn d-btn-sm" @click="load()">Retry</button>
  </div>
  <template v-else>
    <div class="page-head">
      <div>
        <h1>Audit</h1>
        <div class="page-sub">Every outbound model call, newest first</div>
      </div>
      <div class="head-right">
        <div class="head-row">
          <div class="figures">
            <div><span class="lbl">Calls</span><span class="fig-line"><b class="num">{{ rows.length }}</b></span></div>
            <div><span class="lbl">Failures</span><span class="fig-line"><b class="num" :class="failures ? 'neg' : ''">{{ failures }}</b></span></div>
            <div><span class="lbl">Tokens</span><span class="fig-line"><b class="num">{{ tokens.toLocaleString("en-IN") }}</b></span></div>
          </div>
        </div>
      </div>
    </div>

    <ScheduleStrip :prompts="prompts" :now="now" :running="running" @reload="load()" />

    <div class="filter-bar">
      <label v-for="f in [
        { name: 'prompt', label: 'Prompt', values: facets.prompts, humanise: true },
        { name: 'channel', label: 'Call', values: facets.calls, humanise: true },
        { name: 'model', label: 'Model', values: facets.models, humanise: true },
        { name: 'review', label: 'Review', values: ['pending', 'approved', 'rejected', 'blocked', 'none'], humanise: false },
      ]" :key="f.name" class="filter"
      >
        <span class="lbl">{{ f.label }}</span>
        <select :aria-label="f.label" :value="(filters as Record<string, string | undefined>)[f.name] ?? ''"
                @change="setFilter(f.name as keyof Filters, ($event.target as HTMLSelectElement).value)">
          <option value="">Any</option>
          <option v-for="v in f.values" :key="v" :value="v">{{ f.humanise ? humanLabel(v) : v }}</option>
        </select>
      </label>
      <a v-if="filtered" class="d-btn d-btn-sm d-btn-ghost" href="/calls">Clear</a>
      <div class="tick">
        <label class="tick-toggle" title="Reload this list on a timer">
          <input v-model="auto" type="checkbox" @change="remember" />
          <span class="tick-dot" aria-hidden="true"></span>
          <span class="tick-word">Auto-refresh</span>
        </label>
        <select v-model.number="every" aria-label="Refresh interval" @change="remember">
          <option v-for="s in INTERVALS" :key="s" :value="s">{{ s === 60 ? "1 min" : `${s} s` }}</option>
        </select>
      </div>
    </div>

    <section class="main-body table-wrap">
      <table class="blotter">
        <thead><tr>
          <th scope="col">Time</th><th scope="col" class="hide-sm">Prompt type</th>
          <th scope="col" class="hide-sm">Call</th><th scope="col">Holding</th>
          <th scope="col">Review</th><th scope="col" class="right"><span class="hide-sm">View</span></th>
        </tr></thead>
        <tbody>
          <template v-for="r in rows" :key="r.id">
            <tr>
              <td class="time" :title="r.ts">{{ dayTime(r.ts) }}</td>
              <td class="hide-sm">{{ promptType(r) }}</td>
              <td class="hide-sm">{{ titleCase(r.endpoint) }}</td>
              <td class="holding">
                <b v-if="holding(r) === 'Whole book'">Whole book</b>
                <b v-else-if="r.instrument_name">{{ r.instrument_name }}</b>
                <span v-else class="muted">{{ holding(r) }}</span>
                <div v-if="holdingSub(r)" class="sub">{{ holdingSub(r) }}</div>
              </td>
              <td class="review-td">
                <div class="review-cell">
                  <span v-if="reviewBadge(r).label" :title="reviewBadge(r).title"
                        v-html="pill(reviewBadge(r).tone, reviewBadge(r).label, reviewBadge(r).title)" />
                </div>
              </td>
              <td class="right">
                <button class="d-btn d-btn-sm d-btn-ghost" type="button"
                        :aria-expanded="open === r.id" :aria-controls="`call-${r.id}`"
                        aria-label="Show the call" @click="open = open === r.id ? null : r.id">
                  {{ open === r.id ? "Hide" : "Show" }}
                </button>
              </td>
            </tr>
            <tr v-if="open === r.id" :id="`call-${r.id}`" class="call-detail">
              <td colspan="6"><CallDrawer :id="r.id" /></td>
            </tr>
          </template>
          <tr v-if="!rows.length">
            <td colspan="6" class="empty">
              <span v-if="filtered">No calls match these filters — <a href="/calls">clear them</a>.</span>
              <span v-else>No calls yet.</span>
            </td>
          </tr>
        </tbody>
      </table>

      <div class="pager">
        <div class="pager-side">
          <button class="d-btn d-btn-sm" type="button" :disabled="offset === 0"
                  @click="page(Math.max(0, offset - limit))">‹ Newer</button>
          <button class="d-btn d-btn-sm" type="button" :disabled="offset + limit >= total"
                  @click="page(offset + limit)">Older ›</button>
        </div>
        <div class="pager-side">
          <span class="sub">
            {{ total === 0 ? "0–0 of 0" : `${offset + 1}–${Math.min(offset + limit, total)} of ${total}` }}
            · page {{ Math.floor(offset / limit) + 1 }} of {{ Math.max(1, Math.ceil(total / limit)) }}
          </span>
          <label class="pager-size sub">Show
            <select :value="String(limit)" aria-label="Calls per page" @change="pageSize(($event.target as HTMLSelectElement).value)">
              <option v-for="n in [25, 50, 100]" :key="n" :value="n">{{ n }} per page</option>
            </select>
          </label>
        </div>
      </div>
    </section>
  </template>
</template>
