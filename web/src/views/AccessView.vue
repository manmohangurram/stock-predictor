<script setup lang="ts">
/**
 * Access: the keys that open /mcp. A key is shown exactly once, at creation, and the page can
 * never show it again — the table lists a prefix so you can tell two keys apart after the fact.
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, auth, type ApiKey } from "../api";
import SettingsRail from "../components/SettingsRail.vue";
import SettingField from "../components/SettingField.vue";
import { histDate } from "../format";

const router = useRouter();
const keys = ref<ApiKey[]>([]);
const error = ref("");
const busy = ref(false);
const name = ref("");
const scope = ref<"read" | "write">("read");
const days = ref("30");
const revealed = ref<{ token: string; key: ApiKey } | null>(null);
const armed = ref<string | null>(null);
const copied = ref(false);

const lifetimes = computed(() => [
  { value: "1", label: "1 day" },
  { value: "7", label: "7 days" },
  { value: "30", label: "30 days" },
  { value: "90", label: "90 days" },
  { value: "365", label: "365 days" },
  { value: "never", label: "Never expires" },
]);

async function load(): Promise<void> {
  try {
    keys.value = (await api.keys()).keys;
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

async function create(): Promise<void> {
  if (!name.value.trim()) {
    error.value = "Give the key a name, so you know which one to revoke.";
    return;
  }
  busy.value = true;
  try {
    const res = await api.createKey({
      name: name.value.trim(),
      scope: scope.value,
      days: days.value === "never" ? null : Number(days.value),
    });
    revealed.value = { token: res.token, key: res.key };
    copied.value = false;
    name.value = "";
    await load();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function copy(): Promise<void> {
  if (!revealed.value) return;
  try {
    await navigator.clipboard.writeText(revealed.value.token);
    copied.value = true;
  } catch {
    copied.value = false;
  }
}

async function revoke(key: ApiKey): Promise<void> {
  if (armed.value !== key.id) {
    armed.value = key.id;
    return;
  }
  try {
    await api.revokeKey(key.id);
    armed.value = null;
    await load();
  } catch (e) {
    error.value = String(e);
  }
}

function expires(k: ApiKey): string {
  if (!k.expires_at) return "never";
  return histDate(k.expires_at);
}

function lastUsed(k: ApiKey): string {
  if (!k.last_used_at) return "never used";
  return histDate(k.last_used_at);
}

const current = ref("");
const next = ref("");
const again = ref("");
const passwordNote = ref("");
const passwordError = ref("");

async function changePassword(): Promise<void> {
  passwordError.value = "";
  passwordNote.value = "";
  if (next.value.length < 10) {
    passwordError.value = "Use at least 10 characters.";
    return;
  }
  if (next.value !== again.value) {
    passwordError.value = "The two passwords do not match.";
    return;
  }
  try {
    await api.changePassword({ current: current.value, next: next.value });
    current.value = "";
    next.value = "";
    again.value = "";
    passwordNote.value = "Password changed. Sign in again with the new one.";
    auth.set(null);
    setTimeout(() => void router.push("/login"), 1800);
  } catch (e) {
    const text = String(e);
    passwordError.value = text.includes("403")
      ? "That is not the current password."
      : text;
  }
}

onMounted(load);
</script>

<template>
  <section class="access">
    <SettingsRail current="access" />
    <header class="pane-head">
      <h2>Access keys</h2>
      <span class="note">for MCP clients</span>
    </header>
    <p class="lede">
      A key opens <code>/mcp</code> and nothing else — the app and its REST API are unchanged.
      A <b>read</b> key sees the book; a <b>write</b> key can also approve decisions, record fills
      and change settings, which means it can move money. The token is shown once and never again.
    </p>

    <div v-if="error" class="error-band" role="alert">{{ error }}</div>

    <form class="trade-form" autocomplete="off" @submit.prevent="create">
      <div class="row">
        <label class="field">
          <span class="lbl">Name</span>
          <input v-model="name" type="text" placeholder="claude desktop" aria-label="Key name" />
        </label>
        <label class="field">
          <span class="lbl">Scope</span>
          <select v-model="scope" aria-label="Key scope">
            <option value="read">read — sees the book</option>
            <option value="write">write — can act on it</option>
          </select>
        </label>
        <label class="field">
          <span class="lbl">Lifetime</span>
          <select v-model="days" aria-label="Key lifetime">
            <option v-for="l in lifetimes" :key="l.value" :value="l.value">{{ l.label }}</option>
          </select>
        </label>
        <button class="d-btn d-btn-sm d-btn-primary" type="submit" :disabled="busy">Create key</button>
      </div>
    </form>

    <div v-if="revealed" class="error-band" role="status">
      <p><b>{{ revealed.key.name }}</b> — copy this now. It is not shown again.</p>
      <p class="token"><code>{{ revealed.token }}</code></p>
      <div class="row">
        <button class="d-btn d-btn-sm" type="button" @click="copy">{{ copied ? "Copied" : "Copy" }}</button>
        <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="revealed = null">Done</button>
      </div>
    </div>

    <div class="table-wrap">
      <table class="blotter">
        <thead>
          <tr>
            <th scope="col">Name</th>
            <th scope="col">Scope</th>
            <th scope="col">Prefix</th>
            <th scope="col" class="hide-narrow">Created</th>
            <th scope="col">Expires</th>
            <th scope="col" class="hide-narrow">Last used</th>
            <th scope="col" class="right"><span class="sr-only">Revoke</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="k in keys" :key="k.id" :class="{ revoked: k.revoked_at }">
            <td>{{ k.name }}</td>
            <td>{{ k.scope }}</td>
            <td class="mono">{{ k.prefix }}…</td>
            <td class="mono hide-narrow">{{ histDate(k.created_at) }}</td>
            <td class="mono">{{ expires(k) }}</td>
            <td class="mono hide-narrow">{{ lastUsed(k) }}</td>
            <td class="right">
              <span v-if="k.revoked_at" class="muted">revoked</span>
              <button v-else class="d-btn d-btn-xs d-btn-ghost" type="button"
                      :class="{ danger: armed === k.id }" @click="revoke(k)">
                {{ armed === k.id ? "Click again to revoke" : "Revoke" }}
              </button>
            </td>
          </tr>
          <tr v-if="keys.length === 0">
            <td colspan="7" class="empty">No keys yet. Create one for the client you want to connect.</td>
          </tr>
        </tbody>
      </table>
    </div>

    <header class="pane-head">
      <h2>Password</h2>
      <span class="note">the one door</span>
    </header>
    <p class="lede">
      Changing it ends every signed-in session, including the one you are using. There is no reset
      by email; if it is lost, it is replaced on the server.
    </p>
    <form class="trade-form" autocomplete="off" @submit.prevent="changePassword">
      <div class="row">
        <SettingField id="current_password" label="Current password" type="password"
          v-model="current" autocomplete="current-password" />
        <SettingField id="new_password" label="New password" type="password"
          v-model="next" autocomplete="new-password" placeholder="10+ characters" />
        <SettingField id="again_password" label="New password again" type="password"
          v-model="again" autocomplete="new-password" />
        <button class="d-btn d-btn-sm d-btn-primary" type="submit">Change password</button>
      </div>
      <div v-if="passwordError" class="error-band" role="alert">{{ passwordError }}</div>
      <p v-if="passwordNote" class="note" role="status">{{ passwordNote }}</p>
    </form>
  </section>
</template>

<style scoped>
.access { display: block; max-width: 100%; min-width: 0; overflow-x: clip; }
.access .table-wrap { max-width: 100%; }
@media (max-width: 760px) {
  .hide-narrow { display: none; }
}
.token code { display: block; overflow-wrap: anywhere; font-family: var(--font-ledger); }
tr.revoked td { color: var(--ink-muted); }
</style>
