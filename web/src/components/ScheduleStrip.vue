<script setup lang="ts">
/**
 * The schedule strip: one card per prompt that decides something, then the button to start one.
 *
 * The cadence and the next due time are computed on the server from the same markers the scheduler
 * uses — the page never re-implements them, or the card would promise a rhythm the loop does not
 * keep.
 */
import { ref } from "vue";
import { api, type ScheduledPrompt } from "../api";
import { dayTime } from "../format";
import { pill } from "./stamp";

const props = defineProps<{
    prompts: ScheduledPrompt[];
    now: string;
    running: number;
}>();
const emit = defineEmits<{ reload: [] }>();

const due = (iso: string | null): string =>
    !iso
        ? "—"
        : Date.parse(iso) <= Date.parse(props.now) + 1000
          ? "due now"
          : dayTime(iso);

const took = (r: NonNullable<ScheduledPrompt["last"]>): string => {
    if (!r.finished_at) return "running";
    const ms = Date.parse(r.finished_at) - Date.parse(r.at);
    if (!Number.isFinite(ms) || ms < 0) return "—";
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
};

const statusOf = (p: ScheduledPrompt): string =>
    !p.last
        ? ""
        : pill(
              p.last.status === "running"
                  ? "running"
                  : p.last.status === "failed"
                    ? "failed"
                    : "done",
              p.last.status,
          );

const resultOf = (p: ScheduledPrompt): string =>
    p.last?.error ?? p.last?.summary ?? "—";

const untilFinished = async (what: string): Promise<void> => {
    for (let i = 0; i < 40; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        try {
            const s = await api.schedule();
            const p = s.prompts.find((x) => x.prompt === what);
            if (p?.last && p.last.status !== "running") {
                status.value = "";
                emit("reload");
                return;
            }
        } catch {
            return;
        }
    }
};

const runningWhat = ref("");
const prompt = ref("plan_stock");
const symbol = ref("");
const status = ref("");

async function start(what: string): Promise<void> {
    if (runningWhat.value) return;
    runningWhat.value = what;
    status.value = "";
    try {
        await api.runNow(
            what,
            what === "decide" ? symbol.value.trim() || undefined : undefined,
        );
        status.value = `${what.replace("_", " ")} started`;
        emit("reload");
        void untilFinished(what);
    } catch (e) {
        status.value = String(e);
    } finally {
        runningWhat.value = "";
    }
}
</script>

<template>
    <div class="runs">
        <article
            v-for="p in prompts"
            :key="p.prompt"
            class="run"
            :data-state="p.last?.status ?? 'none'"
        >
            <div class="t">
                <span>{{ p.last ? p.last.trigger : "never run" }}</span>
                <span v-html="statusOf(p)" />
            </div>
            <h4>{{ p.label }}</h4>
            <div class="kv2 desk-only">
                <span>Last ran</span
                ><b>{{ p.last ? dayTime(p.last.at) : "—" }}</b>
            </div>
            <div class="kv2 desk-only">
                <span>Next due</span><b>{{ due(p.next_due_at) }}</b>
            </div>
            <div class="kv2 only-mobile">
                <span>Last / next</span>
                <b
                    >{{ p.last ? dayTime(p.last.at) : "—" }} ·
                    {{ due(p.next_due_at) }}</b
                >
            </div>
            <div class="kv2">
                <span>Result</span>
                <b class="trunc" :title="resultOf(p)">{{ resultOf(p) }}</b>
            </div>
            <div class="kv2 desk-only">
                <span>Duration</span><b>{{ p.last ? took(p.last) : "—" }}</b>
            </div>
            <p class="rule">{{ p.rule }}</p>
            <button
                v-if="p.last?.status !== 'running'"
                class="mini"
                type="button"
                :disabled="!!runningWhat"
                @click="start(p.prompt)"
            >
                {{ runningWhat === p.prompt ? "Starting…" : "Run again" }}
            </button>
            <button v-else class="mini" type="button" disabled>Working…</button>
        </article>

        <article class="run add">
            <div class="t"><span>Manual</span></div>
            <h4>Run now</h4>
            <form class="run-now" @submit.prevent="start(prompt)">
                <select v-model="prompt" name="prompt" aria-label="What to run">
                    <option value="plan_stock">Stock plan</option>
                    <option value="plan_fund">Fund plan</option>
                    <option value="plan_trade">Trade plan</option>
                    <option value="poll">A poll</option>
                    <option value="decide">One instrument</option>
                </select>
                <input
                    v-model="symbol"
                    name="symbol"
                    placeholder="INFY.NS"
                    aria-label="Symbol"
                />
                <button
                    class="d-btn d-btn-sm d-btn-primary"
                    type="submit"
                    :disabled="!!runningWhat"
                >
                    Run
                </button>
            </form>
            <span class="help" role="status">{{
                status || (running ? `${running} running now` : "")
            }}</span>
        </article>
    </div>
</template>

<style scoped>
.runs {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
    gap: 12px;
    margin-bottom: 14px;
}
.run {
    display: flex;
    flex-direction: column;
    background: var(--paper-raised);
    border: 1px solid var(--rule-subtle);
    border-radius: var(--radius-card);
    padding: 14px;
    box-shadow: var(--shadow-card);
    min-width: 0;
}
.t {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    font-size: 9px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
h4 {
    margin: 6px 0 9px;
    font-size: 14px;
    font-weight: 650;
    letter-spacing: -0.01em;
}
.kv2 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    padding: 6px 0;
    border-bottom: 1px dotted var(--rule);
    font-size: 12px;
}
.kv2 span {
    color: var(--ink-tertiary);
    flex: none;
}
.kv2 b {
    font-family: var(--font-ledger);
    font-size: 12px;
    font-weight: 500;
    text-align: right;
    min-width: 0;
}
.kv2 .trunc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 62%;
}
.rule {
    margin: 9px 0 12px;
    font-size: 10.5px;
    line-height: 1.45;
    color: var(--ink-tertiary);
}
.mini {
    margin-top: auto;
    align-self: flex-start;
    font: inherit;
    font-size: 11px;
    line-height: 1;
    height: 28px;
    padding: 0 11px;
    border: 1px solid var(--rule);
    border-radius: var(--radius-pill);
    background: transparent;
    color: var(--ink-secondary);
    cursor: pointer;
}
.mini:hover {
    color: var(--ink);
    border-color: var(--rule-strong);
}
.mini:disabled {
    opacity: 0.5;
    cursor: default;
}
.run-now {
    display: grid;
    gap: 8px;
}
.run-now .d-btn {
    justify-self: start;
}
.desk-only {
    display: flex;
}
.only-mobile {
    display: none;
}
.add .help {
    margin-top: 8px;
    font-size: 11px;
    color: var(--ink-tertiary);
}
@media (max-width: 700px) {
    .runs {
        gap: 10px;
    }
    .run {
        padding: 12px;
    }
    .desk-only {
        display: none;
    }
    .only-mobile {
        display: flex;
    }
}
</style>
