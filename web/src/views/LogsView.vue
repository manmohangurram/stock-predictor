<script setup lang="ts">
/**
 * Logs: every call an MCP client made on the book, newest first, with the key it used.
 *
 * Arguments are stored with secrets already redacted. This is a reading page — nothing here
 * changes the book.
 */
import { onMounted, ref } from "vue";
import { Terminal } from "lucide-vue-next";
import { api, type McpCall } from "../api";
import { dayTime } from "../format";
import SettingsRail from "../components/SettingsRail.vue";
import AppEmpty from "../components/ui/AppEmpty.vue";
import { useAdmin } from "../admin";

const { username: adminName } = useAdmin();
const calls = ref<McpCall[]>([]);
const limit = ref(200);
const error = ref("");

async function load(): Promise<void> {
    try {
        calls.value = (await api.mcpCalls(limit.value)).calls;
        error.value = "";
    } catch (e) {
        error.value = String(e);
    }
}

onMounted(load);
</script>

<template>
    <div class="settings-head">
        <header class="pg-top">
            <div class="pg-main">
                <div class="pg-eyebrow">Configuration</div>
                <h1 class="pg-title">Settings</h1>
                <div class="pg-sub">Everything an MCP client asked the book to do</div>
            </div>
            <div class="pg-side">
                <div class="pg-kpis">
                    <div>
                        <span>Signed in</span><b>{{ adminName }}</b>
                    </div>
                    <div>
                        <span>Calls</span><b class="num">{{ calls.length }}</b>
                    </div>
                </div>
            </div>
        </header>
    </div>

    <div class="settings">
        <SettingsRail current="logs" />

        <main class="settings-pane" role="tabpanel" id="panel-logs">
            <div class="formcard">
                <div class="cardhead">
                    <div>
                        <h3>Tool calls</h3>
                        <p class="lead">
                            Newest first, with the key it used. Arguments are stored with secrets
                            already redacted.
                        </p>
                    </div>
                    <label class="note">
                        last
                        <select
                            v-model.number="limit"
                            aria-label="How many calls to list"
                            @change="load"
                        >
                            <option :value="50">50</option>
                            <option :value="200">200</option>
                            <option :value="500">500</option>
                        </select>
                    </label>
                </div>

                <div v-if="error" class="error-band" role="alert">{{ error }}</div>

                <table class="logs-table">
                    <thead>
                        <tr>
                            <th scope="col" class="when">When</th>
                            <th scope="col" class="key">Key</th>
                            <th scope="col" class="tool hide-narrow">Tool</th>
                            <th scope="col" class="args hide-narrow">Arguments</th>
                            <th scope="col">Result</th>
                            <th scope="col" class="r ms">ms</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr v-for="call in calls" :key="call.id">
                            <td class="mono when">{{ dayTime(call.ts) }}</td>
                            <td class="key">
                                {{ call.key_name || call.prefix || "removed key" }}
                            </td>
                            <td class="mono tool hide-narrow">{{ call.tool }}</td>
                            <td class="mono args hide-narrow">{{ call.args_json }}</td>
                            <td>
                                <span v-if="call.status === 'ok'" class="ok">ok</span>
                                <span v-else class="bad">{{ call.error || "error" }}</span>
                            </td>
                            <td class="r mono ms">{{ call.ms }}</td>
                        </tr>
                        <tr v-if="calls.length === 0">
                            <td colspan="6">
                                <AppEmpty
                                    compact
                                    :icon="Terminal"
                                    text="No tool calls yet. Connect a client to /mcp and they appear here."
                                />
                            </td>
                        </tr>
                    </tbody>
                </table>
            </div>
        </main>
    </div>
</template>

<style scoped>
.logs-table {
    width: calc(100% + 44px);
    border-collapse: collapse;
    font-size: 12px;
    margin: 14px -22px 0;
}
.logs-table th {
    font-size: 8.5px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    text-align: left;
    padding: 7px 10px;
    background: var(--paper-soft);
    font-weight: 700;
    white-space: nowrap;
}
.logs-table td {
    padding: 9px 10px;
    border-top: 1px solid var(--rule-subtle);
    vertical-align: top;
    overflow-wrap: anywhere;
}
.logs-table th:first-child,
.logs-table td:first-child {
    padding-left: 22px;
}
.logs-table th:last-child,
.logs-table td:last-child {
    padding-right: 22px;
}
.logs-table th.r,
.logs-table td.r {
    text-align: right;
}
.logs-table .when,
.logs-table .key,
.logs-table .tool,
.logs-table .ms {
    width: 1%;
    white-space: nowrap;
}
.logs-table .args {
    font-size: 10.5px;
    color: var(--ink-secondary);
}
.logs-table .ok {
    color: var(--green);
}
.logs-table .bad {
    color: var(--stamp-oxide);
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
@media (max-width: 1080px) {
    .hide-narrow {
        display: none;
    }
}
@media (max-width: 900px) {
    .logs-table {
        width: calc(100% + 32px);
        margin: 12px -16px 0;
    }
    .logs-table th:first-child,
    .logs-table td:first-child {
        padding-left: 16px;
    }
    .logs-table th:last-child,
    .logs-table td:last-child {
        padding-right: 16px;
    }
}
</style>
