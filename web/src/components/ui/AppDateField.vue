<script setup lang="ts">
/** A date field: the board's text input, plus a calendar button that opens the native picker. */
import { computed, ref, watch } from "vue";
import { Calendar } from "lucide-vue-next";
import { longDay, parseDay } from "../../format";

const props = withDefaults(
    defineProps<{
        modelValue: string;
        label?: string;
        name?: string;
        id?: string;
        max?: string;
        hint?: string;
        placeholder?: string;
        disabled?: boolean;
    }>(),
    {
        label: "Date",
        name: "at",
        id: undefined,
        max: "",
        hint: "Use 22 Nov 2026",
        placeholder: "22 Nov 2026",
        disabled: false,
    },
);

const emit = defineEmits<{ "update:modelValue": [string] }>();

const text = ref(props.modelValue ? longDay(props.modelValue) : "");
const touched = ref(false);
const native = ref<HTMLInputElement | null>(null);

const iso = computed(() => parseDay(text.value));
const invalid = computed(() => iso.value === null || (!!props.max && iso.value > props.max));

watch(
    () => props.modelValue,
    (value) => {
        if ((parseDay(text.value) ?? "") === value) return;
        text.value = value ? longDay(value) : "";
    },
);

function onInput(value: string): void {
    text.value = value;
    touched.value = true;
    emit("update:modelValue", parseDay(value) ?? "");
}

function openPicker(): void {
    const el = native.value;
    if (!el) return;
    try {
        el.showPicker();
    } catch {
        el.focus();
        el.click();
    }
}

function onPick(event: Event): void {
    const value = (event.target as HTMLInputElement).value;
    if (!value) return;
    text.value = longDay(value);
    touched.value = true;
    emit("update:modelValue", value);
}
</script>

<template>
    <label class="fld appdate">
        <span>{{ label }}</span>
        <div class="appdate-line">
            <input
                :id="id"
                :name="name"
                type="text"
                inputmode="text"
                autocomplete="off"
                spellcheck="false"
                :value="text"
                :placeholder="placeholder"
                :class="{ bad: invalid && touched }"
                :disabled="disabled"
                @input="onInput(($event.target as HTMLInputElement).value)"
            />
            <button
                type="button"
                class="appdate-btn"
                :aria-label="`Open calendar for ${label.toLowerCase()}`"
                :disabled="disabled"
                @click="openPicker"
            >
                <Calendar :size="15" :stroke-width="1.75" aria-hidden="true" />
            </button>
            <input
                ref="native"
                class="appdate-native"
                type="date"
                tabindex="-1"
                aria-hidden="true"
                :value="iso ?? ''"
                :max="max || undefined"
                :disabled="disabled"
                @change="onPick"
            />
        </div>
        <em v-if="invalid && touched" class="appdate-hint">{{ hint }}</em>
    </label>
</template>

<style scoped>
.appdate-line {
    position: relative;
    display: block;
}
.appdate-line > input[type="text"] {
    width: 100%;
    padding-right: 34px;
}
.appdate-line > input.bad {
    border-color: var(--stamp-oxide);
    background: var(--stamp-oxide-bg);
}
.appdate-btn {
    position: absolute;
    top: 1px;
    right: 1px;
    bottom: 1px;
    width: 32px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: 0 calc(var(--radius) - 1px) calc(var(--radius) - 1px) 0;
    background: transparent;
    color: var(--ink-tertiary);
    cursor: pointer;
}
.appdate-btn:hover {
    color: var(--ink);
    background: var(--paper);
}
.appdate-btn:disabled {
    color: var(--ink-muted);
    cursor: not-allowed;
}
.appdate-native {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 1px;
    height: 1px;
    padding: 0;
    border: 0;
    opacity: 0;
    pointer-events: none;
}
.appdate-hint {
    font-size: 10px;
    font-style: normal;
    color: var(--stamp-oxide);
}
</style>
