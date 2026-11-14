import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import App from "./App.vue";
import LedgerView from "./views/LedgerView.vue";
import InstrumentView from "./views/InstrumentView.vue";
import CallsView from "./views/CallsView.vue";
import LegacyView from "./views/LegacyView.vue";
import { startAutoRefresh } from "./renderer";

/**
 * Vue owns the app element and the URL. Until the last page is ported, every route renders through
 * `LegacyView`, which hands the markup to the plain renderers in `renderer.ts`.
 */
const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", component: LedgerView },
    { path: "/i/:symbol", component: InstrumentView },
    { path: "/calls", component: CallsView },
    // Everything not yet ported goes through the seam.
    { path: "/:pathMatch(.*)*", component: LegacyView },
  ],
});

createApp(App).use(router).mount("#app");
startAutoRefresh();
