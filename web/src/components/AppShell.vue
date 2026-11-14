<script setup lang="ts">
/**
 * The chrome every page sits in: a narrow rail on the left, the page on the right. It never
 * scrolls; only the page column does. Below 900px the rail becomes a top strip.
 *
 * The market state lives at the foot of the rail, and on a phone under the page title, so the
 * whole app is in one frame whether the reader is at a desk or on a bus.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import { api, type Market } from "../api";
import { railBadges } from "../stores/rail";

const route = useRoute();
const links = [
  { id: "ledger", label: "Ledger", href: "/", group: "work" },
  { id: "audit", label: "Audit", href: "/calls", group: "work" },
  { id: "settings", label: "Settings", href: "/settings", group: "setup" },
  { id: "archived", label: "Archived", href: "/archived", group: "setup" },
] as const;

const active = computed(() => {
  const p = route.path;
  if (p.startsWith("/calls")) return "audit";
  if (p.startsWith("/settings")) return "settings";
  if (p.startsWith("/archived")) return "archived";
  return "ledger";
});

const badge = (id: string): string =>
  id === "ledger" ? railBadges.ledger : "";

const today = new Date().toLocaleDateString("en-IN", {
  day: "2-digit",
  month: "short",
  year: "numeric",
});

const market = ref<Market | null>(null);
let timer: number | undefined;

async function loadMarket(): Promise<void> {
  try {
    market.value = await api.market();
  } catch {
    market.value = null;
  }
}

const marketLabel = computed(() => {
  const m = market.value;
  if (!m) return "";
  if (m.state === "open") return "Market open";
  return m.state === "holiday" ? "Market shut" : "Market closed";
});

const marketWhen = computed(() => {
  const m = market.value;
  if (!m) return "";
  const at = m.state === "open" ? m.closes_at : m.next_open_at;
  if (!at) return "";
  const when = new Date(at).toLocaleString("en-IN", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${m.state === "open" ? "Closes" : "Opens"} ${when}`;
});

onMounted(() => {
  void loadMarket();
  timer = window.setInterval(() => void loadMarket(), 60_000);
});
onBeforeUnmount(() => window.clearInterval(timer));
</script>

<template>
  <div class="app">
    <nav class="rail" aria-label="Sections">
      <div class="rail-brand">
        <img src="/favicon.svg" width="28" height="28" alt="" aria-hidden="true" />
        <b>Stock Predictor</b>
        <span>{{ today }}</span>
      </div>
      <template v-for="group in ['work', 'setup']" :key="group">
        <a
          v-for="l in links.filter((x) => x.group === group)"
          :key="l.id"
          :href="l.href"
          :class="{ on: active === l.id }"
          :aria-current="active === l.id ? 'page' : undefined"
        >
          <span>{{ l.label }}</span>
          <span v-if="badge(l.id)" class="rail-badge">{{ badge(l.id) }}</span>
        </a>
        <div v-if="group === 'work'" class="rail-sep" role="presentation"></div>
      </template>
      <div
        v-if="market"
        class="rail-market"
        role="status"
        aria-live="polite"
        :data-state="market.state"
        :title="`${marketLabel} · ${market.reason} · ${market.session}`"
      >
        <span class="lbl"><span class="dot" aria-hidden="true"></span>{{ marketLabel }}</span>
        <span class="reason">{{ marketWhen }}</span>
      </div>
    </nav>
    <main class="main">
      <!-- On a phone the rail is a bar with no foot, so the market state moves here, under the
           page title: one status, two homes, never both. -->
      <div
        v-if="market"
        class="shell-market"
        role="status"
        aria-live="polite"
        :data-state="market.state"
      >
        <span class="lbl"><span class="dot" aria-hidden="true"></span>{{ marketLabel }}</span>
        <span class="reason">{{ marketWhen }}</span>
      </div>
      <slot />
    </main>
  </div>
</template>
