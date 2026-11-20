<script setup lang="ts">
import { api, auth } from "../api";

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

async function signOut(): Promise<void> {
  try {
    await api.authLogout();
  } finally {
    auth.set(null);
    window.location.href = "/login";
  }
}
</script>

<style scoped>
.sign-out {
  margin-left: auto;
  align-self: center;
  background: none;
  border: 0;
  padding: 4px 8px;
  color: var(--ink-muted);
  font: inherit;
  cursor: pointer;
}
.sign-out:hover { color: var(--ink); }
</style>

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
      <button class="sign-out" type="button" @click="signOut">Sign out</button>
    </nav>
  </div>
</template>
