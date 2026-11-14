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

const props = defineProps<{ prompts: ScheduledPrompt[]; now: string; running: number }>();
const emit = defineEmits<{ reload: [] }>();

const due = (iso: string | null): string =>
  !iso ? "—" : Date.parse(iso) <= Date.parse(props.now) + 1000 ? "due now" : dayTime(iso);

const took = (r: NonNullable<ScheduledPrompt["last"]>): string => {
  if (!r.finished_at) return "running";
  const ms = Date.parse(r.finished_at) - Date.parse(r.at);
  if (!Number.isFinite(ms) || ms < 0) return "—";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
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
    await api.runNow(what, what === "decide" ? symbol.value.trim() || undefined : undefined);
    status.value = `${what.replace("_", " ")} started`;
    emit("reload");
  } catch (e) {
    status.value = String(e);
  } finally {
    runningWhat.value = "";
  }
}
</script>

<template>
  <div class="runs-strip">
    <article
      v-for="p in prompts"
      :key="p.prompt"
      class="rcard"
      :data-state="p.last?.status ?? 'none'"
    >
      <div class="rcard-top">
        <span v-html="pill(p.last ? (p.last.status === 'running' ? 'running' : p.last.status === 'failed' ? 'failed' : 'done') : '', p.last ? p.last.status : 'never run')" />
        <span class="run-when">{{ p.last ? `last ${dayTime(p.last.at)}` : "no run yet" }}</span>
      </div>
      <div class="rcard-title">
        {{ p.label }}
        <span v-if="p.last" class="trigger">{{ p.last.trigger }}{{ p.last.finished_at ? ` · ${took(p.last)}` : "" }}</span>
      </div>
      <div class="rcard-result">{{ p.last?.error ?? p.last?.summary ?? (p.last ? "—" : "Nothing has run yet.") }}</div>
      <div class="rcard-next"><span class="lbl">Next due</span> <b>{{ due(p.next_due_at) }}</b></div>
      <div class="rcard-rule">{{ p.rule }}</div>
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

    <article class="rcard rcard-add">
      <div class="rcard-title">Run now</div>
      <form class="run-now" @submit.prevent="start(prompt)">
        <select v-model="prompt" name="prompt" aria-label="What to run">
          <option value="plan_stock">Stock plan</option>
          <option value="plan_fund">Fund plan</option>
          <option value="plan_trade">Trade plan</option>
          <option value="poll">A poll</option>
          <option value="decide">One instrument</option>
        </select>
        <input v-model="symbol" name="symbol" placeholder="INFY.NS" aria-label="Symbol" />
        <button class="d-btn d-btn-sm d-btn-primary run-now-btn" type="submit" :disabled="!!runningWhat">Run</button>
        <span class="help" role="status">{{ status || (running ? `${running} running now` : "") }}</span>
      </form>
    </article>
  </div>
</template>
