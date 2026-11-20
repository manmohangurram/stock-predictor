<script setup lang="ts">
import { computed, ref } from "vue";
import { api } from "../api";
import SettingField from "./SettingField.vue";

const props = withDefaults(defineProps<{ variant?: "A" | "B" | "C"; symbols?: string[] }>(), {
  variant: "A",
  symbols: () => [],
});

const symbol = ref("");
const armed = ref(false);
const busy = ref(false);
const note = ref("");
const error = ref("");

const hint = computed(() => {
  const list = props.symbols.slice(0, 3);
  return list.length ? `In this book: ${list.join(", ")}` : "The book is empty, so type EMPTY.";
});

const typed = computed(() => symbol.value.trim());
const matches = computed(() => {
  if (props.symbols.length === 0) return typed.value.toUpperCase() === "EMPTY";
  return props.symbols.some((s) => s.toUpperCase() === typed.value.toUpperCase());
});

async function wipe(): Promise<void> {
  error.value = "";
  note.value = "";
  if (!typed.value) {
    error.value = "Type a symbol from this book first.";
    return;
  }
  if (!matches.value) {
    error.value = `${typed.value} is not in this book.`;
    return;
  }
  if (props.variant !== "C" && !armed.value) {
    armed.value = true;
    return;
  }
  busy.value = true;
  try {
    const res = await api.resetBook(typed.value);
    const count = Object.values(res.deleted).reduce((n, v) => n + v, 0);
    note.value = `Deleted ${count} rows. The book is empty; you are still signed in.`;
    symbol.value = "";
    armed.value = false;
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="danger" :class="`v${variant}`" aria-labelledby="danger-title">
    <header class="danger-head">
      <span class="danger-icon" aria-hidden="true">!</span>
      <div class="danger-titles">
        <h2 id="danger-title">Delete everything</h2>
        <p class="danger-sub">Clears the book. The admin and your session stay.</p>
      </div>
      <span class="pill todo">cannot be undone</span>
    </header>

    <template v-if="variant === 'A'">
      <p class="lede">
        Instruments, bars, holdings, fills, the cash ledger, decisions, notifications, runs, call
        records, MCP keys and settings all go. Type the symbol you are willing to lose.
      </p>
      <form class="lock-row" @submit.prevent="wipe">
        <SettingField id="lock_symbol" label="Symbol" v-model="symbol" placeholder="INFY.NS" />
        <button class="d-btn d-btn-sm danger-action" type="submit" :disabled="busy">
          {{ armed ? "Click again to delete" : "Delete everything" }}
        </button>
      </form>
      <p class="hint">{{ hint }}</p>
    </template>

    <template v-else-if="variant === 'B'">
      <p class="lede">
        Two steps: open the confirmation, then type the symbol. Nothing is deleted until both are
        true.
      </p>
      <button v-if="!armed" class="d-btn d-btn-sm danger-action" type="button" @click="armed = true">
        Delete everything…
      </button>
      <form v-else class="step-two" @submit.prevent="wipe">
        <p class="hint">{{ hint }}</p>
        <SettingField id="step_symbol" label="Type the symbol to confirm" v-model="symbol" placeholder="INFY.NS" />
        <div class="step-actions">
          <button class="d-btn d-btn-sm danger-solid" type="submit" :disabled="busy">
            I understand — delete everything
          </button>
          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="armed = false">Cancel</button>
        </div>
      </form>
    </template>

    <template v-else>
      <ul class="what">
        <li><b>Portfolio</b> instruments, holdings, fills, cash ledger</li>
        <li><b>Decisions</b> signals, notifications, plans</li>
        <li><b>Record</b> model calls, MCP tool calls</li>
        <li><b>Access</b> MCP keys, settings</li>
      </ul>
      <form class="check-form" @submit.prevent="wipe">
        <SettingField id="check_symbol" label="Type the symbol you are deleting" v-model="symbol" placeholder="INFY.NS" />
        <p class="hint">{{ hint }}</p>
        <button class="d-btn d-btn-sm danger-solid" type="submit" :disabled="busy || !matches">
          Delete {{ matches ? typed.toUpperCase() : "…" }} and everything else
        </button>
      </form>
    </template>

    <div v-if="error" class="error-band" role="alert">{{ error }}</div>
    <p v-if="note" class="note" role="status">{{ note }}</p>
  </section>
</template>

<style scoped>
.danger {
  margin-top: var(--space-8);
  padding: var(--space-4);
  border: 1px solid var(--stamp-oxide-bg);
  border-radius: var(--radius-lg);
  background: var(--paper-raised);
  display: grid;
  gap: var(--space-3);
}
.danger-head { display: flex; align-items: flex-start; gap: var(--space-3); }
.danger-icon {
  width: 24px; height: 24px; flex: none;
  display: grid; place-items: center;
  border-radius: 50%;
  background: var(--stamp-oxide-bg);
  color: var(--stamp-oxide);
  font-family: var(--font-form);
  font-weight: 700;
}
.danger-titles { flex: 1; min-width: 0; }
.danger h2 {
  margin: 0;
  font-family: var(--font-form);
  font-size: var(--text-lg);
  color: var(--stamp-oxide);
}
.danger-sub { margin: 2px 0 0; font-size: var(--text-sm); color: var(--ink-muted); }
.lede { margin: 0; color: var(--ink-secondary); max-width: 62ch; }
.hint { margin: 0; font-size: var(--text-sm); color: var(--ink-tertiary); }
.lock-row { display: flex; align-items: flex-end; gap: var(--space-3); flex-wrap: wrap; }
.lock-row :deep(.field) { width: 220px; }
.lock-row :deep(.field input) { max-width: none; width: 100%; }
.step-two { display: grid; gap: var(--space-3); max-width: 420px; }
.step-two :deep(.field input) { max-width: none; width: 100%; }
.step-actions { display: flex; gap: var(--space-2); flex-wrap: wrap; }
.what {
  margin: 0;
  padding: 0;
  list-style: none;
  display: grid;
  gap: var(--space-1);
  font-size: var(--text-sm);
  color: var(--ink-secondary);
}
.what b { color: var(--ink); font-family: var(--font-form); margin-right: var(--space-2); }
.check-form { display: grid; gap: var(--space-3); max-width: 420px; }
.check-form :deep(.field input) { max-width: none; width: 100%; }
.danger-action {
  border-color: var(--stamp-oxide);
  color: var(--stamp-oxide);
  background: var(--paper-raised);
}
.danger-action:hover { background: var(--stamp-oxide-bg); }
.danger > .danger-action { justify-self: start; width: auto; }
.check-form .danger-solid { justify-self: start; width: auto; }
.danger-solid {
  background: var(--stamp-oxide);
  border-color: var(--stamp-oxide);
  color: var(--paper-raised);
}
.danger-solid:disabled { opacity: .45; }
</style>
