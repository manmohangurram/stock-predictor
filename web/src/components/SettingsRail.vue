<script setup lang="ts">
/** The settings sections: one card of tabs on a desk, one selector row on a phone. */
import { computed } from "vue";
import { useRouter } from "vue-router";
import { ChevronDown } from "lucide-vue-next";

const props = defineProps<{ current: string }>();
const router = useRouter();

const TABS = [
    { id: "money", label: "Money", hint: "Allowances" },
    { id: "risk", label: "Risk limits", hint: "Limits" },
    { id: "model", label: "Model", hint: "Provider" },
    { id: "notifications", label: "Notifications", hint: "ntfy" },
    { id: "cadence", label: "Cadence", hint: "Polling" },
    { id: "network", label: "Network", hint: "Proxy" },
    { id: "access", label: "Access", hint: "Session, keys" },
    { id: "logs", label: "Logs", hint: "MCP calls" },
];

const here = computed(() => TABS.find((t) => t.id === props.current) ?? TABS[0]);

function go(event: Event): void {
    void router.push(`/settings/${(event.target as HTMLSelectElement).value}`);
}
</script>

<template>
    <nav class="tabs" role="tablist" aria-label="Settings sections">
        <a
            v-for="t in TABS"
            :id="`tab-${t.id}`"
            :key="t.id"
            class="tab"
            :class="{ on: current === t.id }"
            role="tab"
            :href="`/settings/${t.id}`"
            :aria-selected="current === t.id"
            :aria-controls="`panel-${t.id}`"
            >{{ t.label }}</a
        >
    </nav>

    <label class="tabsel">
        <span>{{ here.label }}</span>
        <span class="tabsel-hint">{{ here.hint }}</span>
        <ChevronDown class="tabsel-chev" :size="14" :stroke-width="1.75" aria-hidden="true" />
        <select :value="current" aria-label="Settings section" @change="go">
            <option v-for="t in TABS" :key="t.id" :value="t.id">{{ t.label }}</option>
        </select>
    </label>
</template>

<style scoped>
.tabsel {
    position: relative;
    display: none;
}
@media (max-width: 900px) {
    .tabsel {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 10px;
        background: var(--paper-raised);
        border: 1px solid var(--rule-subtle);
        border-radius: var(--radius-card);
        padding: 11px 14px;
        box-shadow: var(--shadow-card);
        font-size: 13px;
        font-weight: 650;
        color: var(--ink);
    }
    .tabsel select {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        opacity: 0;
        cursor: pointer;
    }
    .tabsel-hint {
        margin-left: auto;
        font-size: 11px;
        font-weight: 400;
        color: var(--ink-tertiary);
    }
    .tabsel-chev {
        color: var(--ink-tertiary);
    }
}
</style>
