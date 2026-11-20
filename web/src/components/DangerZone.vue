<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { api } from "../api";
import SettingField from "./SettingField.vue";

const emit = defineEmits<{ deleted: [count: number] }>();

const dialog = ref<HTMLDialogElement | null>(null);
const symbol = ref("");
const symbols = ref<string[]>([]);
const busy = ref(false);
const armed = ref(false);
const note = ref("");
const error = ref("");
const field = ref<HTMLElement | null>(null);

const placeholder = computed(() => symbols.value[0] ?? "EMPTY");

async function open(): Promise<void> {
  error.value = "";
  note.value = "";
  armed.value = false;
  symbol.value = "";
  dialog.value?.showModal();
  try {
    symbols.value = (await api.resetCandidates()).symbols;
  } catch {
    symbols.value = [];
  }
  await nextTick();
  field.value?.querySelector("input")?.focus();
}

function close(): void {
  dialog.value?.close();
}

function matches(): boolean {
  const typed = symbol.value.trim().toUpperCase();
  if (!typed) return false;
  if (symbols.value.length === 0) return typed === "EMPTY";
  return symbols.value.some((s) => s.toUpperCase() === typed);
}

async function wipe(): Promise<void> {
  error.value = "";
  const typed = symbol.value.trim();
  if (!matches()) {
    error.value = typed ? `${typed} is not in this book.` : "Type a symbol from this book.";
    return;
  }
  if (!armed.value) {
    armed.value = true;
    return;
  }
  busy.value = true;
  try {
    const res = await api.resetBook(typed);
    const count = Object.values(res.deleted).reduce((n, v) => n + v, 0);
    note.value = `Deleted ${count} rows.`;
    close();
    emit("deleted", count);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="delete-book">
    <button class="d-btn d-btn-sm delete-open" type="button" aria-haspopup="dialog" @click="open">
      Delete the book…
    </button>
    <p v-if="note" class="note" role="status">{{ note }}</p>

    <dialog ref="dialog" class="delete-dialog" @close="armed = false" aria-labelledby="delete-title">
      <form method="dialog" class="delete-body" @submit.prevent="wipe">
        <header class="delete-head">
          <span class="delete-icon" aria-hidden="true">!</span>
          <div class="delete-words">
            <h2 id="delete-title">Delete everything</h2>
            <p class="delete-sub">Clears the book. The admin and your session stay.</p>
          </div>
          <span class="pill todo">cannot be undone</span>
        </header>

        <p class="delete-lede">
          Instruments, bars, holdings, fills, the cash ledger, decisions, notifications, runs, call
          records, MCP keys and settings all go. Type the symbol you are willing to lose.
        </p>

        <div ref="field" class="delete-field">
          <SettingField id="delete_symbol" label="Symbol" v-model="symbol" :placeholder="placeholder" />
        </div>

        <div v-if="error" class="error-band" role="alert">{{ error }}</div>

        <div class="delete-foot">
          <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="close">Cancel</button>
          <button class="d-btn d-btn-sm delete-action" type="submit" :disabled="busy">
            {{ armed ? "Click again to delete everything" : "Delete everything" }}
          </button>
        </div>
      </form>
    </dialog>
  </div>
</template>

<style scoped>
.delete-book { display: flex; align-items: center; gap: var(--space-3); flex-wrap: wrap; }
.delete-open { border-color: var(--stamp-oxide); color: var(--stamp-oxide); background: var(--paper-raised); }
.delete-open:hover { background: var(--stamp-oxide-bg); }

.delete-dialog {
  width: min(540px, calc(100vw - 32px));
  padding: 0;
  border: 1px solid var(--rule-strong);
  border-radius: var(--radius-lg);
  background: var(--paper-raised);
  color: var(--ink);
}
.delete-dialog::backdrop { background: oklch(0.2 0.01 60 / 0.45); }

.delete-body {
  display: grid;
  gap: var(--space-4);
  padding: var(--space-6);
  margin: 0;
}

.delete-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.delete-icon {
  flex: none;
  width: 32px;
  height: 32px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  background: var(--stamp-oxide-bg);
  color: var(--stamp-oxide);
  font-family: var(--font-form);
  font-size: var(--text-md);
  font-weight: 700;
  line-height: 1;
}
.delete-words { flex: 1; min-width: 0; }
.delete-words h2 {
  margin: 0;
  font-family: var(--font-form);
  font-size: var(--text-lg);
  line-height: 1.2;
}
.delete-sub { margin: 2px 0 0; font-size: var(--text-sm); color: var(--ink-muted); }
.delete-head .pill { flex: none; }

.delete-lede { margin: 0; color: var(--ink-secondary); font-size: var(--text-sm); }
.delete-field { display: grid; gap: var(--space-2); }
.delete-body :deep(.field) { width: 100%; }
.delete-body :deep(.field input) { width: 100%; max-width: none; }
.delete-body .error-band { margin: 0; }

.delete-foot { display: flex; justify-content: flex-end; gap: var(--space-2); }
.delete-action { border-color: var(--stamp-oxide); color: var(--stamp-oxide); background: var(--paper-raised); }
.delete-action:hover { background: var(--stamp-oxide-bg); }

@media (max-width: 480px) {
  .delete-body { padding: var(--space-4); }
  .delete-head { flex-wrap: wrap; }
  .delete-head .pill { order: 3; margin-left: 44px; }
}
</style>
