<script setup lang="ts">
/** An empty state: one icon, one sentence about what is absent, and the next step when there is one. */
import type { Component } from "vue";

withDefaults(
    defineProps<{
        text: string;
        icon?: Component;
        compact?: boolean;
    }>(),
    { icon: undefined, compact: false },
);
</script>

<template>
    <div class="empty-state" :class="{ compact }" role="status">
        <span v-if="icon" class="empty-art" aria-hidden="true">
            <component :is="icon" :size="17" :stroke-width="1.6" />
        </span>
        <p class="empty-text">{{ text }}</p>
        <div v-if="$slots.default" class="empty-act"><slot /></div>
    </div>
</template>

<style scoped>
.empty-state {
    display: grid;
    justify-items: center;
    align-content: center;
    gap: 10px;
    padding: 42px 20px 46px;
    text-align: center;
}
.empty-state.compact {
    padding: 24px 16px 26px;
    gap: 8px;
}
.empty-art {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    border: 1px solid var(--rule-subtle);
    background: var(--paper-inset);
    color: var(--ink-tertiary);
}
.empty-state.compact .empty-art {
    width: 28px;
    height: 28px;
}
.empty-text {
    margin: 0;
    max-width: 46ch;
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-tertiary);
}
.empty-state.compact .empty-text {
    font-size: 11.5px;
}
.empty-act {
    margin-top: 2px;
}
</style>
