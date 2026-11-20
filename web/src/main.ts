import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import App from "./App.vue";
import { auth } from "./api";
import LedgerView from "./views/LedgerView.vue";
import InstrumentView from "./views/InstrumentView.vue";
import CallsView from "./views/CallsView.vue";
import ArchivedView from "./views/ArchivedView.vue";
import AccessView from "./views/AccessView.vue";
import LoginView from "./views/LoginView.vue";
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
    { path: "/login", component: LoginView },
  { path: "/settings", component: SettingsView },
    { path: "/settings/access", component: AccessView },
  { path: "/settings/:tab", component: SettingsView },
    // Anything else is the ledger: an unknown URL lands somewhere useful.
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});

let opened = false;

// The access token lives in memory, so a fresh load asks the refresh cookie for one before the
// first page renders; a refused refresh sends the visitor to the sign-in screen.
router.beforeEach(async (to) => {
  if (to.path === "/login") return true;
  if (!opened) {
    opened = true;
    if (!auth.token()) await auth.refresh();
  }
  return auth.token() ? true : "/login";
});

createApp(App).use(router).mount("#app");
