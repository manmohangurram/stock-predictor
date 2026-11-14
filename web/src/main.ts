import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import App from "./App.vue";
import LegacyView from "./views/LegacyView.vue";
import { startAutoRefresh } from "./renderer";

/**
 * Vue owns the app element and the URL. Until the last page is ported, every route renders through
 * `LegacyView`, which hands the markup to the plain renderers in `renderer.ts`.
 */
const router = createRouter({
  history: createWebHistory(),
  routes: [{ path: "/:pathMatch(.*)*", component: LegacyView }],
});

createApp(App).use(router).mount("#app");
startAutoRefresh();
