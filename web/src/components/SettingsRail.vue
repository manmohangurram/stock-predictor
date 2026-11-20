<script setup lang="ts">
const TABS = [
  { id: "money", label: "Money" },
  { id: "risk", label: "Risk limits" },
  { id: "model", label: "Model" },
  { id: "notifications", label: "Notifications" },
  { id: "cadence", label: "Cadence" },
  { id: "network", label: "Network" },
  { id: "access", label: "Access" },
];

defineProps<{ current: string }>();
</script>

<template>
  <div class="tabs-row">
    <nav class="settings-rail" role="tablist" aria-label="Settings sections">
      <a
        v-for="t in TABS"
        :key="t.id"
        role="tab"
        :id="`tab-${t.id}`"
        :href="`/settings/${t.id}`"
        :aria-selected="current === t.id"
        :aria-controls="`panel-${t.id}`"
        :class="{ on: current === t.id }"
      >{{ t.label }}</a>
    </nav>
  </div>
</template>
