import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import App from "./App.vue";
import LedgerView from "./views/LedgerView.vue";
import InstrumentView from "./views/InstrumentView.vue";
import CallsView from "./views/CallsView.vue";
import ArchivedView from "./views/ArchivedView.vue";
import SettingsView from "./views/SettingsView.vue";


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
    { path: "/archived", component: ArchivedView },
    { path: "/settings", component: SettingsView },
    { path: "/settings/:tab", component: SettingsView },
    // Anything else is the ledger: an unknown URL lands somewhere useful.
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});

createApp(App).use(router).mount("#app");
