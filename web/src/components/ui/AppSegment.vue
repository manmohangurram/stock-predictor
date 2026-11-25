<script setup lang="ts">
/** A segmented choice: one option is on at a time, and each one says whether it is. */
withDefaults(
    defineProps<{
        modelValue: string;
        options: { value: string; label: string }[];
        label?: string;
    }>(),
    { label: "" },
);

const emit = defineEmits<{ "update:modelValue": [string] }>();
</script>

<template>
    <div class="seg" role="group" :aria-label="label || undefined">
        <button
            v-for="o in options"
            :key="o.value"
            type="button"
            class="seg-btn"
            :class="{ on: o.value === modelValue }"
            :aria-pressed="o.value === modelValue"
            @click="emit('update:modelValue', o.value)"
        >
            {{ o.label }}
        </button>
    </div>
</template>

<style scoped>
.seg {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
}
.seg-btn {
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--rule);
    border-radius: var(--radius-pill);
    background: var(--paper-raised);
    color: var(--ink-secondary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    white-space: nowrap;
}
.seg-btn:hover {
    color: var(--ink);
    border-color: var(--rule-strong);
}
.seg-btn.on {
    background: var(--ink);
    border-color: var(--ink);
    color: var(--paper-raised);
    font-weight: 600;
}
.seg-btn:focus-visible {
    outline: 2px solid var(--rule-focus);
    outline-offset: 1px;
}
@media (max-width: 900px) {
    .seg {
        gap: 9px;
    }
    .seg-btn {
        height: 24px;
        padding: 0 8px;
        font-size: 10px;
    }
}
</style>
