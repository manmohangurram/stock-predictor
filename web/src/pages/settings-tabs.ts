/** The settings page is six tabs; each is its own route, its own form and its own stamp. */
export type Tab = "money" | "risk" | "model" | "notifications" | "cadence" | "network";

export interface TabSpec {
  id: Tab;
  label: string;
  title: string;
  lede: string;
  /** What the rail shows on the right: a count of fields, or a state word. */
  badge?: (s: { llm_key_configured: boolean; ntfy_topic: string; runtime: { proxy_configured: boolean } }) => string;
}

export const TABS: TabSpec[] = [
  {
    id: "money",
    label: "Money",
    title: "Money",
    lede: "Three purses, each granted monthly and each expiring: long-term stocks, mutual funds, and the short-term book. They never share money.",
  },
  {
    id: "risk",
    label: "Risk limits",
    title: "Risk limits",
    lede: "The band bounds any single stock's profit target. A stock may raise its floor inside the band; the maximum always wins.",
  },
  {
    id: "model",
    label: "Model",
    title: "Model",
    lede: "Which chat API writes the paragraph. Without a key the rules run alone and no call is made.",
    badge: (s) => (s.llm_key_configured ? "set" : "no key"),
  },
  {
    id: "notifications",
    label: "Notifications",
    title: "Notifications",
    lede: "The push channel. A topic name is the only address: anyone who knows it can read it, so make it long and random.",
    badge: (s) => (s.ntfy_topic.trim() ? "set" : "off"),
  },
  {
    id: "cadence",
    label: "Cadence",
    title: "Cadence",
    lede: "How often the app wakes, and when it is allowed to spend a model call. Changes apply on the next tick — no restart.",
  },
  {
    id: "network",
    label: "Network",
    title: "Network",
    lede: "One proxy for all outbound traffic — market data, the model and notifications. The environment is never consulted.",
    badge: (s) => (s.runtime.proxy_configured ? "set" : "direct"),
  },
];

export const isTab = (v: string): v is Tab => TABS.some((t) => t.id === v);

export function tabRail(active: Tab, s: Parameters<NonNullable<TabSpec["badge"]>>[0]): string {
  return `<nav class="settings-rail" role="tablist" aria-label="Settings sections">
    ${TABS.map((t) => {
      const badge = t.badge?.(s) ?? "";
      return `<a role="tab" id="tab-${t.id}" href="/settings/${t.id}"
                 aria-selected="${t.id === active}" aria-controls="panel-${t.id}"
                 tabindex="${t.id === active ? 0 : -1}"
                 class="${t.id === active ? "on" : ""}">
        <span>${t.label}</span>${badge ? `<span class="rail-badge">${badge}</span>` : ""}
      </a>`;
    }).join("")}
  </nav>`;
}
