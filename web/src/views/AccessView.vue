<script setup lang="ts">
/**
 * Access: the keys that open /mcp, the session on this browser, and the secrets at rest.
 *
 * A key is shown exactly once, at creation, so the form is a dialog and the table keeps only a
 * prefix. Every live session is listed; one switch ends them all.
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, auth, type ApiKey, type SessionInfo, type Settings } from "../api";
import SettingsRail from "../components/SettingsRail.vue";
import SettingField from "../components/SettingField.vue";
import { KeyRound, LogOut, User } from "lucide-vue-next";
import { histDate } from "../format";
import AppEmpty from "../components/ui/AppEmpty.vue";
import { useAdmin } from "../admin";

const router = useRouter();
const keys = ref<ApiKey[]>([]);
const sessions = ref<SessionInfo[]>([]);
const error = ref("");
const busy = ref(false);
const busySessions = ref(false);
const name = ref("");
const scope = ref<"read" | "write">("read");
const days = ref("30");
const revealed = ref<{ token: string; key: ApiKey } | null>(null);
const armed = ref<string | null>(null);
const copied = ref(false);
const config = ref<Settings | null>(null);

const createDialog = ref<HTMLDialogElement | null>(null);
const passwordDialog = ref<HTMLDialogElement | null>(null);
const nameDialog = ref<HTMLDialogElement | null>(null);
const { username: adminName, setName } = useAdmin();

const activeKeys = computed(() => keys.value.filter((k) => !k.revoked_at).length);

const lifetimes = computed(() => [
    { value: "1", label: "1 day" },
    { value: "7", label: "7 days" },
    { value: "30", label: "30 days" },
    { value: "90", label: "90 days" },
    { value: "365", label: "365 days" },
    { value: "never", label: "Never expires" },
]);

/** A user agent as a person names it: browser first, platform second. */
function device(ua: string): string {
    if (!ua.trim()) return "Unknown device";
    const browser = /Edg\//.test(ua)
        ? "Edge"
        : /OPR\//.test(ua)
          ? "Opera"
          : /Chrome\//.test(ua)
            ? "Chrome"
            : /Firefox\//.test(ua)
              ? "Firefox"
              : /Safari\//.test(ua)
                ? "Safari"
                : "Browser";
    const os = /Windows/.test(ua)
        ? "Windows"
        : /Mac OS X|Macintosh/.test(ua)
          ? "macOS"
          : /Android/.test(ua)
            ? "Android"
            : /iPhone|iPad/.test(ua)
              ? "iOS"
              : /Linux/.test(ua)
                ? "Linux"
                : "device";
    return `${browser} · ${os}`;
}

async function load(): Promise<void> {
    try {
        const [k, s] = await Promise.all([
            api.keys(),
            api.sessions().catch(() => ({ sessions: [] })),
        ]);
        keys.value = k.keys;
        sessions.value = s.sessions;
        error.value = "";
    } catch (e) {
        error.value = String(e);
    }
}

function openCreate(): void {
    name.value = "";
    scope.value = "read";
    days.value = "30";
    error.value = "";
    const d = createDialog.value;
    if (d && !d.open) d.showModal();
}

function closeCreate(): void {
    const d = createDialog.value;
    if (d?.open) d.close();
}

function openPassword(): void {
    current.value = "";
    next.value = "";
    again.value = "";
    passwordError.value = "";
    passwordNote.value = "";
    const d = passwordDialog.value;
    if (d && !d.open) d.showModal();
}

function closePassword(): void {
    const d = passwordDialog.value;
    if (d?.open) d.close();
}

async function signOutAll(): Promise<void> {
    busySessions.value = true;
    try {
        await api.revokeAllSessions();
    } finally {
        auth.set(null);
        window.location.href = "/login";
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
        closeCreate();
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
        passwordError.value = text.includes("403") ? "That is not the current password." : text;
    }
}

const newName = ref("");
const namePassword = ref("");
const nameError = ref("");
const nameNote = ref("");
const busyName = ref(false);

function openName(): void {
    newName.value = adminName.value;
    namePassword.value = "";
    nameError.value = "";
    nameNote.value = "";
    const d = nameDialog.value;
    if (d && !d.open) d.showModal();
}

function closeName(): void {
    const d = nameDialog.value;
    if (d?.open) d.close();
}

async function renameUser(): Promise<void> {
    nameError.value = "";
    nameNote.value = "";
    const wanted = newName.value.trim();
    if (!wanted) {
        nameError.value = "Enter a name.";
        return;
    }
    if (wanted.length > 64) {
        nameError.value = "Use at most 64 characters.";
        return;
    }
    busyName.value = true;
    try {
        const res = await api.changeUsername({
            username: wanted,
            password: namePassword.value,
        });
        setName(res.username);
        newName.value = res.username;
        namePassword.value = "";
        nameNote.value = "Signed in as " + res.username + " from now on.";
    } catch (e) {
        const text = String(e);
        nameError.value = text.includes("403") ? "That is not the current password." : text;
    } finally {
        busyName.value = false;
    }
}

onMounted(async () => {
    await load();
    config.value = await api.settings().catch(() => null);
});
</script>

<template>
    <div class="settings-head">
        <header class="pg-top">
            <div class="pg-main">
                <div class="pg-eyebrow">Configuration</div>
                <h1 class="pg-title">Settings</h1>
                <div class="pg-sub">Keys that open /mcp, and the session on this browser</div>
            </div>
            <div class="pg-side">
                <div class="pg-kpis">
                    <div>
                        <span>Signed in</span><b>{{ adminName }}</b>
                    </div>
                    <div>
                        <span>Active keys</span><b class="num">{{ activeKeys }}</b>
                    </div>
                </div>
            </div>
        </header>
    </div>

    <div class="settings">
        <SettingsRail current="access" />

        <main class="settings-pane access" role="tabpanel" id="panel-access">
            <div class="formcard">
                <h3>Session</h3>
                <p class="lead">
                    This browser is signed in as {{ adminName }}. Signing out ends every live
                    session below, on every device.
                </p>
                <div class="sess">
                    <div class="who">
                        <span class="avatar">
                            <User :size="15" :stroke-width="1.75" aria-hidden="true" />
                        </span>
                        <div>
                            <b>{{ adminName }}</b><span class="hint">local session</span>
                        </div>
                    </div>
                    <button
                        class="d-btn d-btn-sm d-btn-ghost danger"
                        type="button"
                        :disabled="busySessions"
                        @click="signOutAll"
                    >
                        <LogOut :size="14" :stroke-width="1.75" aria-hidden="true" />
                        {{ busySessions ? "Signing out…" : "Sign out all" }}
                    </button>
                </div>
                <ul class="sesslist">
                    <li v-for="s in sessions" :key="s.id">
                        <span class="dev">{{ device(s.user_agent) }}</span>
                        <span class="meta">signed in {{ histDate(s.created_at) }}</span>
                        <span v-if="s.current" class="tag current">this device</span>
                        <span v-else class="meta right">expires {{ histDate(s.expires_at) }}</span>
                    </li>
                    <li v-if="!sessions.length" class="meta">No live sessions.</li>
                </ul>
            </div>

            <div class="formcard">
                <div class="cardhead">
                    <div>
                        <h3>API keys</h3>
                        <p class="lead">
                            A key is shown once, at creation. The table keeps a prefix so two keys
                            stay distinguishable. A
                            <b>read</b> key sees the book; a <b>write</b> key can also act on it.
                        </p>
                    </div>
                    <button class="d-btn d-btn-sm d-btn-primary" type="button" @click="openCreate">
                        Create a key
                    </button>
                </div>

                <div v-if="error" class="error-band" role="alert">
                    {{ error }}
                </div>

                <div v-if="revealed" class="error-band token" role="status">
                    <p>
                        <b>{{ revealed.key.name }}</b> — copy this now. It is not shown again.
                    </p>
                    <p>
                        <code>{{ revealed.token }}</code>
                    </p>
                    <div class="row wide">
                        <button class="d-btn d-btn-sm" type="button" @click="copy">
                            {{ copied ? "Copied" : "Copy" }}
                        </button>
                        <button
                            class="d-btn d-btn-sm d-btn-ghost"
                            type="button"
                            @click="revealed = null"
                        >
                            Done
                        </button>
                    </div>
                </div>

                <table class="access-table">
                    <thead>
                        <tr>
                            <th scope="col">Name</th>
                            <th scope="col">Scope</th>
                            <th scope="col">Prefix</th>
                            <th scope="col">Expires</th>
                            <th scope="col" class="r">
                                <span class="sr-only">Revoke</span>
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr v-for="k in keys" :key="k.id" :class="{ revoked: k.revoked_at }">
                            <td>
                                <b>{{ k.name }}</b>
                            </td>
                            <td>
                                <span class="tag">{{ k.scope }}</span>
                            </td>
                            <td class="mono">{{ k.prefix }}…</td>
                            <td class="mono">{{ expires(k) }}</td>
                            <td class="r">
                                <span v-if="k.revoked_at" class="muted">revoked</span>
                                <button
                                    v-else
                                    class="d-btn d-btn-xs d-btn-ghost"
                                    type="button"
                                    :class="{ danger: armed === k.id }"
                                    @click="revoke(k)"
                                >
                                    {{ armed === k.id ? "Click again to revoke" : "Revoke" }}
                                </button>
                            </td>
                        </tr>
                        <tr v-if="keys.length === 0">
                            <td colspan="5">
                                <AppEmpty
                                    compact
                                    :icon="KeyRound"
                                    text="No keys yet. Create one for the client you want to connect."
                                />
                            </td>
                        </tr>
                    </tbody>
                </table>
                <p class="hint foot-hint">
                    Revoke is armed on the first click, confirmed on the second.
                </p>
            </div>

            <div class="formcard">
                <h3>Secrets at rest</h3>
                <p class="lead">
                    Provider keys, the ntfy token and the proxy password are stored encrypted with
                    the
                    <span class="mono">enc:v1</span> prefix. Losing
                    <span class="mono">SECRET_KEY</span> means re-entering them.
                </p>
                <div class="kv2">
                    <span>Provider key</span>
                    <b>{{ config?.llm_key_configured ? "enc:v1 · stored" : "not set" }}</b>
                </div>
                <div class="kv2">
                    <span>ntfy token</span>
                    <b>{{ config?.ntfy_token_configured ? "enc:v1 · stored" : "not set" }}</b>
                </div>
                <div class="kv2">
                    <span>Proxy password</span>
                    <b>{{
                        config?.runtime.proxy_pass_configured ? "enc:v1 · stored" : "not set"
                    }}</b>
                </div>
                <div class="kv2"><span>Everything else</span><b>a settings row</b></div>
            </div>

            <div class="formcard">
                <div class="cardhead">
                    <div>
                        <h3>Sign-in name</h3>
                        <p class="lead">
                            The name you type at the door. There is one admin, so this is the only
                            name the book knows.
                        </p>
                    </div>
                    <button class="d-btn d-btn-sm" type="button" @click="openName">
                        Change username
                    </button>
                </div>
                <div class="kv2">
                    <span>Current name</span><b>{{ adminName }}</b>
                </div>
            </div>

            <div class="formcard">
                <div class="cardhead">
                    <div>
                        <h3>Password</h3>
                        <p class="lead">
                            Changing it ends every signed-in session, including the one you are
                            using. There is no reset by email; if it is lost, it is replaced on the
                            server.
                        </p>
                    </div>
                    <button class="d-btn d-btn-sm" type="button" @click="openPassword">
                        Change password
                    </button>
                </div>
            </div>
        </main>
    </div>

    <dialog ref="createDialog" class="appmodal" aria-labelledby="create-key-title">
        <form autocomplete="off" @submit.prevent="create">
            <header class="appmodal-head">
                <div>
                    <h2 id="create-key-title">Create a key</h2>
                    <p>Shown once, at creation.</p>
                </div>
                <span class="kbd">esc</span>
            </header>
            <div class="appmodal-body">
                <div class="formgrid">
                    <label class="field">
                        <span class="lbl">Name</span>
                        <input
                            v-model="name"
                            type="text"
                            placeholder="claude desktop"
                            aria-label="Key name"
                            autofocus
                        />
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
                            <option v-for="l in lifetimes" :key="l.value" :value="l.value">
                                {{ l.label }}
                            </option>
                        </select>
                    </label>
                </div>
                <p v-if="error" class="error-band" role="alert">{{ error }}</p>
            </div>
            <footer class="appmodal-foot">
                <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="closeCreate">
                    Cancel
                </button>
                <button class="d-btn d-btn-sm d-btn-primary" type="submit" :disabled="busy">
                    {{ busy ? "Creating…" : "Create a key" }}
                </button>
            </footer>
        </form>
    </dialog>

    <dialog ref="passwordDialog" class="appmodal" aria-labelledby="password-title">
        <form autocomplete="off" @submit.prevent="changePassword">
            <header class="appmodal-head">
                <div>
                    <h2 id="password-title">Change password</h2>
                    <p>Every signed-in session ends.</p>
                </div>
                <span class="kbd">esc</span>
            </header>
            <div class="appmodal-body">
                <div class="formgrid">
                    <SettingField
                        id="current_password"
                        label="Current password"
                        type="password"
                        v-model="current"
                        autocomplete="current-password"
                    />
                    <SettingField
                        id="new_password"
                        label="New password"
                        type="password"
                        v-model="next"
                        autocomplete="new-password"
                        placeholder="10+ characters"
                    />
                    <SettingField
                        id="again_password"
                        label="New password again"
                        type="password"
                        v-model="again"
                        autocomplete="new-password"
                    />
                </div>
                <div v-if="passwordError" class="error-band" role="alert">
                    {{ passwordError }}
                </div>
                <p v-if="passwordNote" class="note" role="status">
                    {{ passwordNote }}
                </p>
            </div>
            <footer class="appmodal-foot">
                <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="closePassword">
                    Cancel
                </button>
                <button class="d-btn d-btn-sm d-btn-primary" type="submit">Change password</button>
            </footer>
        </form>
    </dialog>

    <dialog ref="nameDialog" class="appmodal" aria-labelledby="name-title">
        <form autocomplete="off" @submit.prevent="renameUser">
            <header class="appmodal-head">
                <div>
                    <h2 id="name-title">Change username</h2>
                    <p>This browser stays signed in.</p>
                </div>
                <span class="kbd">esc</span>
            </header>
            <div class="appmodal-body">
                <div class="formgrid">
                    <SettingField
                        id="new_username"
                        label="New username"
                        v-model="newName"
                        autocomplete="username"
                    />
                    <SettingField
                        id="name_password"
                        label="Current password"
                        type="password"
                        v-model="namePassword"
                        autocomplete="current-password"
                    />
                </div>
                <div v-if="nameError" class="error-band" role="alert">
                    {{ nameError }}
                </div>
                <p v-if="nameNote" class="note" role="status">
                    {{ nameNote }}
                </p>
            </div>
            <footer class="appmodal-foot">
                <button class="d-btn d-btn-sm d-btn-ghost" type="button" @click="closeName">
                    Cancel
                </button>
                <button class="d-btn d-btn-sm d-btn-primary" type="submit" :disabled="busyName">
                    {{ busyName ? "Changing…" : "Change username" }}
                </button>
            </footer>
        </form>
    </dialog>
</template>

<style scoped>
.access {
    display: block;
    max-width: 100%;
    min-width: 0;
}
.access-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    margin-top: 16px;
}
.access-table th {
    font-size: 8.5px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    text-align: left;
    padding: 7px 8px;
    background: var(--paper-soft);
    font-weight: 700;
    white-space: nowrap;
}
.access-table td {
    padding: 9px 8px;
    border-top: 1px solid var(--rule-subtle);
    vertical-align: middle;
    overflow-wrap: anywhere;
}
.access-table th:first-child,
.access-table td:first-child {
    padding-left: 0;
}
.access-table th:last-child,
.access-table td:last-child {
    padding-right: 0;
}
.access-table th.r,
.access-table td.r {
    text-align: right;
}
.cardhead {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
}
.cardhead .lead {
    margin-bottom: 0;
}
.foot-hint {
    margin: 10px 0 0;
    font-size: 11px;
    color: var(--ink-tertiary);
}
.token code {
    display: block;
    overflow-wrap: anywhere;
    font-family: var(--font-ledger);
}
tr.revoked td {
    color: var(--ink-muted);
}
.sesslist {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
}
.sesslist li {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 7px 2px;
    border-bottom: 1px dotted var(--rule);
    font-size: 11.5px;
}
.sesslist li:last-child {
    border-bottom: 0;
}
.sesslist .dev {
    font-weight: 600;
    color: var(--ink);
}
.sesslist .meta {
    color: var(--ink-tertiary);
}
.sesslist .right {
    margin-left: auto;
}
.sesslist .tag.current {
    margin-left: auto;
    background: var(--brand-soft);
    color: var(--brand);
    border-color: transparent;
}
</style>
