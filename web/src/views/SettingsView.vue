<script setup lang="ts">
/**
 * Settings: six routed tabs, each stamped on its own.
 *
 * Every value is a database row the next tick reads, so the page never restarts the app. The tab is
 * a URL, a draft lives per tab, and a secret is only sent when one was typed — an empty box means
 * "keep the stored key".
 */
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { api, type Settings } from "../api";
import { humanDuration } from "../lib/duration";
import { money } from "../format";
import { pill } from "../components/stamp";
import SettingField from "../components/SettingField.vue";
import SettingsRail from "../components/SettingsRail.vue";
import { patchFor, seed, type Tab } from "../settings-patch";
import { useAdmin } from "../admin";

const route = useRoute();
const router = useRouter();
const { username: adminName } = useAdmin();
const data = ref<Settings | null>(null);
const values = reactive<Record<string, string>>({});
const saved = ref<Record<string, string>>({});
const status = ref("");
const error = ref("");
const notifyStatus = ref("");
const proxyStatus = ref("");
const busy = ref(false);
const activeKeys = ref(0);

const TABS: { id: Tab; title: string; card: string; lede: string }[] = [
  { id: "money", title: "Money", card: "Allowances", lede: "Three purses, each granted monthly and each expiring: long-term stocks, mutual funds, and the short-term book. They never share money." },
  { id: "risk", title: "Risk limits", card: "Limits", lede: "What a decision may be, per purse. The validator refuses a proposal outside these numbers rather than shrinking it." },
  { id: "model", title: "Model", card: "Provider", lede: "Which chat API writes the paragraph. Without a key the rules run alone and no call is made." },
  { id: "notifications", title: "Notifications", card: "ntfy", lede: "Where a decided trade reaches your phone. One topic, one app; nothing is pushed when it is blank." },
  { id: "cadence", title: "Cadence", card: "Polling", lede: "How often the app wakes, and when it is allowed to spend a model call. Changes apply on the next tick — no restart." },
  { id: "network", title: "Network", card: "Proxy", lede: "The proxy every outbound request goes through. The password is stored apart from the URL so it can be masked like a key." },
];

const tab = computed<Tab>(() => {
  const want = String(route.params.tab ?? "money");
  return (TABS.some((t) => t.id === want) ? want : "money") as Tab;
});
const spec = computed(() => TABS.find((t) => t.id === tab.value)!);
const dirty = computed(() => JSON.stringify(values) !== JSON.stringify(saved.value));

function seedTab(): void {
  if (!data.value) return;
  const seeded = seed(tab.value, data.value);
  for (const k of Object.keys(values)) delete values[k];
  Object.assign(values, seeded);
  saved.value = { ...seeded };
  status.value = "";
}

async function load(): Promise<void> {
  try {
    const [s, k] = await Promise.all([
      api.settings(),
      api.keys().catch(() => ({ keys: [] })),
    ]);
    data.value = s;
    activeKeys.value = k.keys.filter((x) => !x.revoked_at).length;
    error.value = "";
    seedTab();
  } catch (e) {
    error.value = String(e);
  }
}

async function save(): Promise<void> {
  const result = patchFor(tab.value, values);
  if ("error" in result) {
    status.value = result.error;
    return;
  }
  busy.value = true;
  status.value = "Saving…";
  try {
    const res = await api.putSettings(result.patch);
    if (!res.ok) {
      status.value = String(res.body.error ?? "rejected");
      if (tab.value === "money" && res.body.error === "below_committed") {
        status.value = `Committed is ${res.body.committed} — the budget cannot go below it.`;
      }
      return;
    }
    await load();
    status.value = "Saved.";
  } catch (e) {
    status.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function testNotify(): Promise<void> {
  notifyStatus.value = "Sending…";
  const res = await api.testNotify();
  notifyStatus.value = res.ok ? "Sent. Check your phone." : `Failed: ${res.body.error ?? "unknown error"}`;
}

async function testProxy(): Promise<void> {
  proxyStatus.value = "Sending one request through it…";
  try {
    const r = await api.testProxy();
    proxyStatus.value = r.ok
      ? `Reached the provider · HTTP ${r.status} · ${r.millis} ms`
      : `Failed after ${r.millis} ms — ${r.error}`;
  } catch (e) {
    proxyStatus.value = String(e);
  }
}

const tabLabel = (t: (typeof TABS)[number]): string => {
  const s = data.value;
  if (!s) return t.title;
  if (t.id === "model") return s.llm_key_configured ? t.title : `${t.title} · no key`;
  if (t.id === "notifications") return s.ntfy_topic ? t.title : `${t.title} · off`;
  if (t.id === "network") return s.runtime.proxy_configured ? t.title : `${t.title} · direct`;
  return t.title;
};

onMounted(load);
watch(tab, seedTab);
watch(
  () => route.params.tab,
  (want) => {
    if (!want) void router.replace(`/settings/${tab.value}`);
  },
);

const LIMITS = [
  ["min_profit_pct", "Minimum profit", "0.5", "%"],
  ["max_profit_pct", "Maximum profit", "0.5", "%"],
  ["min_trade_amount", "Smallest trade", "1", ""],
  ["max_trade_amount", "Largest trade", "1", ""],
  ["max_instrument_pct", "One stock may be", "1", "%"],
] as const;

const every = (key: "poll_secs" | "off_hours_secs"): string =>
  `wakes every ${humanDuration(Number(values[key] ?? 0))}`;
</script>

<template>
  <div v-if="error" class="error-band">Something failed: {{ error }}</div>
  <template v-else-if="data">
    <div class="settings-head">
      <header class="pg-top">
        <div class="pg-main">
          <div class="pg-eyebrow">Configuration</div>
          <h1 class="pg-title">Settings</h1>
          <div class="pg-sub">Every value is a database row, applied on the next tick</div>
        </div>
        <div class="pg-side">
          <div class="pg-kpis">
            <div><span>Signed in</span><b>{{ adminName }}</b></div>
            <div><span>Active keys</span><b class="num">{{ activeKeys }}</b></div>
          </div>
        </div>
      </header>
    </div>

    <div class="settings">
      <SettingsRail :current="tab" />

      <main class="settings-pane" role="tabpanel" :id="`panel-${tab}`">
        <form autocomplete="off" @submit.prevent="save">
          <div class="formcard">
            <h3>{{ spec.card }}</h3>
            <p class="lead">{{ spec.lede }}</p>
            <div class="formgrid">
        <template v-if="tab === 'money'">
          <SettingField id="budget" label="Monthly allowance · long-term stocks" :unit="data.currency" type="number" step="1"
            v-model="values.budget"
            :help="`Bought and held for six months or more. ${money(data.available, 0)} free this month. Unspent expires.`" />
          <SettingField id="mf_budget" label="Monthly allowance · mutual funds" :unit="data.currency" type="number" step="1"
            v-model="values.mf_budget"
            :help="`A fund is a six-month-plus commitment: a buy settles in about a day, but selling returns the cash only after about four days. ${money(data.mf_available, 0)} free this month.`" />
          <SettingField id="trading_budget" label="Monthly allowance · short-term" :unit="data.currency" type="number" step="1"
            v-model="values.trading_budget"
            :help="`One hour to one week, stocks only. ${money(data.trading_available, 0)} free this month.`" />
          <div class="field">
            <label class="lbl" for="currency">Currency</label>
            <select id="currency" v-model="values.currency" name="currency" :disabled="data.currency_locked">
              <option v-for="c in ['INR', 'USD', 'EUR', 'GBP']" :key="c" :value="c">{{ c }}</option>
            </select>
            <p class="help">{{ data.currency_locked
              ? "Currency is fixed once the ledger has entries."
              : "Locked permanently after the first recorded transaction." }}</p>
          </div>
        </template>

        <template v-else-if="tab === 'risk'">
          <table class="limits-table wide">
            <thead><tr><th scope="col">Limit</th><th scope="col">Long-term limits</th><th scope="col">Short-term limits</th></tr></thead>
            <tbody>
              <tr v-for="[key, label, step, unit] in LIMITS" :key="key">
                <th scope="row">{{ label }} {{ unit }}</th>
                <td v-for="mode in ['long', 'trade']" :key="mode">
                  <input :id="`${mode}_${key}`" :name="`${mode}_${key}`" type="number" min="0" :step="step"
                         v-model="values[`${mode}_${key}`]" :aria-label="`${label}, ${mode === 'long' ? 'long-term' : 'trading'}`" />
                </td>
              </tr>
            </tbody>
          </table>
          <p class="help wide">
            A take-profit below this is rejected, and so is a target above the maximum: no fantasy numbers.
            Percentages are of that pot, so the caps keep their meaning when the budget changes; a stock's own
            profit expectation must fall inside the band, and the band's maximum always wins.
            Amounts are in {{ data.currency }}.
          </p>
          <p class="help wide">
            A smallest trade below this is not worth its fees; a largest trade caps one fill. One stock may be at
            most its share of the pot.
          </p>
        </template>

        <template v-else-if="tab === 'model'">
          <div class="field">
            <label class="lbl" for="llm_provider">Provider</label>
            <select id="llm_provider" v-model="values.llm_provider" name="llm_provider">
              <option value="deepseek">DeepSeek</option>
              <option value="openai">OpenAI</option>
              <option value="gemini">Google Gemini</option>
              <option value="custom">OpenAI-compatible (OpenRouter, Groq, Ollama…)</option>
            </select>
          </div>
          <SettingField id="llm_model" label="Model" v-model="values.llm_model"
            placeholder="default for the provider" help="Leave blank to use the provider's default." />
          <SettingField id="llm_base_url" label="Base URL" v-model="values.llm_base_url"
            placeholder="default for the provider"
            help="Change it for a proxy or a local Ollama: <code>http://localhost:11434/v1</code>." />
          <SettingField id="llm_key" label="API key" type="password" v-model="values.llm_key"
            :placeholder="data.llm_key_configured ? '•••••••• configured — leave blank to keep' : 'key for the selected provider'"
            :help="data.llm_key_configured
              ? 'Configured. The key is never shown again, here or over the API.'
              : 'Without a key the rules run alone and no model call is made.'" />
        </template>

        <template v-else-if="tab === 'notifications'">
          <SettingField id="ntfy_topic" label="Topic" v-model="values.ntfy_topic"
            placeholder="stock-predictor-7f3k9m2q"
            help="Subscribe to this topic in the ntfy app. Anyone who knows the name can read it." />
          <SettingField id="ntfy_url" label="Server" v-model="values.ntfy_url"
            placeholder="https://ntfy.sh" help="Use your own ntfy instance here if you self-host one." />
          <SettingField id="ntfy_token" label="Access token" type="password" v-model="values.ntfy_token"
            :placeholder="data.ntfy_token_configured ? '•••••••• configured — leave blank to keep' : 'tk_… (only for a protected topic)'"
            :help="data.ntfy_token_configured
              ? 'Configured. The token is never shown again, here or over the API.'
              : 'Optional. Needed only when the topic requires authentication.'" />
          <div class="row wide">
            <button class="d-btn d-btn-sm" type="button" @click="testNotify">Send test</button>
            <span class="help" role="status">{{ notifyStatus }}</span>
          </div>
        </template>

        <template v-else-if="tab === 'cadence'">
          <SettingField id="poll_secs" label="Poll while the market is open" :unit="data.currency === 'INR' ? 'seconds' : 'seconds'"
            type="number" step="60" v-model="values.poll_secs"
            :help="`09:00–16:00 IST, Monday to Friday — ${every('poll_secs')}.`" />
          <SettingField id="off_hours_secs" label="Poll when the market is closed" unit="seconds"
            type="number" step="60" v-model="values.off_hours_secs"
            :help="`Outside that window, only to catch headlines — ${every('off_hours_secs')}.`" />
          <SettingField id="prompt_window_hours" label="Tape in each prompt" unit="hours" type="number" step="1"
            v-model="values.prompt_window_hours"
            help="Hours of 15-minute bars the model sees — the biggest lever on token cost." />
          <SettingField id="daily_cap" label="Model calls per day, at most" unit="calls" type="number" step="1"
            v-model="values.daily_cap" help="Past it, the rules run alone for the rest of the day." />
          <SettingField id="min_price_move" label="Call only if the price moved" :unit="data.currency"
            type="number" step="0.5" v-model="values.min_price_move"
            help="Since the last call. A new headline always earns a call regardless." />

          <h3 class="settings-group wide">What the short-term scan may offer</h3>
          <SettingField id="scan_min_price" label="Price floor" :unit="data.currency" type="number" step="1"
            v-model="values.scan_min_price"
            help="Below this, a single tick is a large part of the move. Micro-caps at ₹1–₹4 are not tradeable at this size." />
          <SettingField id="scan_min_turnover" label="Turnover floor, per day" unit="crore" type="number" step="1"
            v-model="values.scan_min_turnover"
            help="The day's traded value a candidate must reach, in whole crore — 1 crore is 1,00,00,000. An order from this purse has to be a rounding error against it." />
          <SettingField id="scan_locked_pct" label="Locked at the circuit at" unit="% up" type="number" step="0.5"
            v-model="values.scan_locked_pct"
            help="A stock this far up is pinned at its 20% band: no fill is available at any price you name." />
        </template>

        <template v-else-if="tab === 'network'">
          <SettingField id="proxy" label="Proxy address" v-model="values.proxy"
            :placeholder="data.runtime.proxy_configured ? data.runtime.proxy_host : 'http://10.0.0.2:3128'"
            help="Host and port only, no credentials. Leave blank to clear the proxy and connect directly. The environment's <code>HTTP_PROXY</code> is never used." />
          <SettingField id="proxy_user" label="Proxy username" v-model="values.proxy_user"
            placeholder="optional" help="Sent as basic proxy authentication, together with the password below." />
          <SettingField id="proxy_pass" label="Proxy password" type="password" v-model="values.proxy_pass"
            :placeholder="data.runtime.proxy_pass_configured ? '•••••••• configured — leave blank to keep' : 'optional'"
            :help="data.runtime.proxy_pass_configured
              ? 'Configured. The password is never shown again, here or over the API.'
              : 'Stored apart from the URL, so it can be masked like a key.'" />
          <SettingField id="proxy_bypass" label="Bypass for" v-model="values.proxy_bypass"
            placeholder="localhost,127.0.0.1,…"
            help="Comma-separated hosts that skip the proxy. There is no <code>NO_PROXY</code> here, on purpose." />
          <div class="row wide">
            <button class="d-btn d-btn-sm" type="button" @click="testProxy">Test</button>
            <span class="help" role="status">{{ proxyStatus }}</span>
          </div>
        </template>

            </div>
          </div>
          <div class="savebar">
            <span v-html="pill(dirty ? 'todo' : 'ok', dirty ? 'unsaved' : 'saved')" />
            <span class="hint grow" role="status">{{ status }}</span>
            <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="seedTab">Discard</button>
            <button class="d-btn d-btn-sm d-btn-primary" type="submit" :disabled="busy">Save</button>
          </div>
        </form>
      </main>
    </div>
  </template>
</template>
