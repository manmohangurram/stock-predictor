<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { api } from "../api";
import SettingField from "./SettingField.vue";

const props = defineProps<{ symbol: string; name: string; disabled?: boolean }>();
const emit = defineEmits<{ deleted: [symbol: string] }>();

const dialog = ref<HTMLDialogElement | null>(null);
const typed = ref("");
const busy = ref(false);
const error = ref("");
const field = ref<HTMLElement | null>(null);

const matches = computed(() => typed.value.trim().toUpperCase() === props.symbol.toUpperCase());

async function open(): Promise<void> {
    error.value = "";
    typed.value = "";
    dialog.value?.showModal();
    await nextTick();
    field.value?.querySelector("input")?.focus();
}

function close(): void {
    dialog.value?.close();
}

async function remove(): Promise<void> {
    if (!matches.value) {
        error.value = "Type the symbol exactly to confirm.";
        return;
    }
    busy.value = true;
    error.value = "";
    try {
        await api.deleteInstrument(props.symbol);
        close();
        emit("deleted", props.symbol);
    } catch (e) {
        error.value = String(e);
    } finally {
        busy.value = false;
    }
}
</script>

<template>
    <div class="delete-instrument">
        <button
            class="d-btn d-btn-sm d-btn-ghost delete-open"
            type="button"
            aria-haspopup="dialog"
            :disabled="disabled"
            @click="open"
        >
            Delete
        </button>

        <dialog ref="dialog" class="delete-dialog" aria-labelledby="delete-instrument-title">
            <form method="dialog" class="delete-body" @submit.prevent="remove">
                <header class="delete-head">
                    <span class="delete-icon" aria-hidden="true">!</span>
                    <div class="delete-words">
                        <h2 id="delete-instrument-title">Delete {{ name }}</h2>
                        <p class="delete-sub">{{ symbol }}</p>
                    </div>
                    <span class="pill todo">cannot be undone</span>
                </header>

                <p class="delete-lede">
                    Its prices, fills, decisions and headlines go, and the cash those fills moved
                    comes back to the pot. Every other instrument is left alone.
                </p>

                <div ref="field" class="delete-field">
                    <SettingField
                        id="delete_instrument_symbol"
                        :label="`Type ${symbol} to confirm`"
                        v-model="typed"
                        :placeholder="symbol"
                    />
                </div>

                <div v-if="error" class="error-band" role="alert">{{ error }}</div>

                <div class="delete-foot">
                    <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="close">
                        Cancel
                    </button>
                    <button
                        class="d-btn d-btn-sm delete-action"
                        type="submit"
                        :disabled="busy || !matches"
                    >
                        {{ busy ? "Deleting…" : "Delete instrument" }}
                    </button>
                </div>
            </form>
        </dialog>
    </div>
</template>

<style scoped>
.delete-instrument {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
}
.delete-open:not(:disabled) {
    border-color: var(--stamp-oxide);
    color: var(--stamp-oxide);
    background: var(--paper-raised);
}
.delete-open:not(:disabled):hover {
    background: var(--stamp-oxide-bg);
}
.delete-open:disabled {
    cursor: not-allowed;
}

.delete-dialog {
    width: min(540px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--rule-strong);
    border-radius: var(--radius-lg);
    background: var(--paper-raised);
    color: var(--ink);
}
.delete-dialog::backdrop {
    background: oklch(0.2 0.01 60 / 0.45);
}

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
.delete-words {
    flex: 1;
    min-width: 0;
}
.delete-words h2 {
    margin: 0;
    font-family: var(--font-form);
    font-size: var(--text-lg);
    line-height: 1.2;
}
.delete-sub {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--ink-muted);
}
.delete-head .pill {
    flex: none;
}

.delete-lede {
    margin: 0;
    color: var(--ink-secondary);
    font-size: var(--text-sm);
}
.delete-field {
    display: grid;
    gap: var(--space-2);
}
.delete-body :deep(.field) {
    width: 100%;
}
.delete-body :deep(.field input) {
    width: 100%;
    max-width: none;
}
.delete-body .error-band {
    margin: 0;
}

.delete-foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
}
.delete-action {
    border-color: var(--stamp-oxide);
    color: var(--stamp-oxide);
    background: var(--paper-raised);
}
.delete-action:hover {
    background: var(--stamp-oxide-bg);
}
.delete-action:disabled {
    opacity: 0.6;
}

@media (max-width: 480px) {
    .delete-body {
        padding: var(--space-4);
    }
    .delete-head {
        display: grid;
        grid-template-columns: 32px minmax(0, 1fr);
        gap: var(--space-2) var(--space-3);
        align-items: start;
    }
    .delete-icon {
        grid-column: 1;
        grid-row: 1;
    }
    .delete-words {
        grid-column: 2;
        grid-row: 1;
    }
    .delete-head .pill {
        grid-column: 1 / -1;
        grid-row: 2;
        justify-self: start;
    }
    .delete-words h2 {
        font-size: var(--text-lg);
    }
}
</style>
