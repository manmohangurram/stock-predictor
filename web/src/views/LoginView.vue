<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Eye, EyeOff, Lock } from "lucide-vue-next";
import { api, auth } from "../api";
import SettingField from "../components/SettingField.vue";

const router = useRouter();
const mode = ref<"login" | "setup">("login");
const username = ref("");
const password = ref("");
const confirm = ref("");
const error = ref("");
const busy = ref(false);
const ready = ref(false);
const reveal = ref(false);
const band = ref<HTMLElement | null>(null);

onMounted(async () => {
    try {
        const status = await api.authStatus();
        mode.value = status.needs_setup ? "setup" : "login";
        if (status.username) username.value = status.username;
    } catch (e) {
        error.value = String(e);
    } finally {
        ready.value = true;
    }
});

async function fail(message: string): Promise<void> {
    error.value = message;
    await nextTick();
    band.value?.focus();
}

async function submit(): Promise<void> {
    error.value = "";
    if (!username.value.trim()) return fail("Enter the username.");
    if (password.value.length < (mode.value === "setup" ? 10 : 1)) {
        return fail(mode.value === "setup" ? "Use at least 10 characters." : "Enter the password.");
    }
    if (mode.value === "setup" && password.value !== confirm.value) {
        return fail("The two passwords do not match.");
    }
    busy.value = true;
    try {
        const body = {
            username: username.value.trim(),
            password: password.value,
        };
        const res = mode.value === "setup" ? await api.authSetup(body) : await api.authLogin(body);
        auth.set(res.access_token);
        await router.push("/");
    } catch (e) {
        const text = String(e);
        if (text.includes("401")) await fail("That password does not match.");
        else if (text.includes("409"))
            await fail("This instance already has an admin. Sign in instead.");
        else await fail(text);
    } finally {
        busy.value = false;
    }
}
</script>

<template>
    <section class="signin">
        <div class="signin-left">
            <div class="signin-glow" aria-hidden="true"></div>

            <div class="signin-brand">
                <span class="authmark">SP</span>
                <b>Stock Predictor</b>
            </div>

            <div class="signin-pitch">
                <h2>A book, a fund purse, and the decision behind every fill.</h2>
                <p class="wide-copy">
                    Local SQLite. NSE and AMFI. The model proposes; you sign off each leg before
                    anything posts.
                </p>
                <p class="narrow-copy">Local SQLite · NSE and AMFI · you sign off each leg</p>
                <div class="facts">
                    <div><b>5</b>destinations</div>
                    <div><b>8</b>settings tabs</div>
                    <div><b>1</b>decision log</div>
                </div>
            </div>
        </div>

        <div class="signin-right">
            <div class="authcard">
                <span class="authmark card-mark">SP</span>
                <h1>{{ mode === "setup" ? "Set up the admin" : "Sign in" }}</h1>
                <p class="lead">
                    {{
                        mode === "setup"
                            ? "This book has no password yet. Choose one now — it is the only door, and it cannot be recovered."
                            : "Use the credentials you set on first run."
                    }}
                </p>

                <form autocomplete="on" @submit.prevent="submit">
                    <SettingField
                        id="username"
                        label="Username"
                        v-model="username"
                        autocomplete="username"
                        :disabled="!ready"
                    />
                    <SettingField
                        id="password"
                        label="Password"
                        :type="reveal ? 'text' : 'password'"
                        v-model="password"
                        :autocomplete="mode === 'setup' ? 'new-password' : 'current-password'"
                        :placeholder="mode === 'setup' ? '10+ characters' : ''"
                        autofocus
                    >
                        <template #trail>
                            <button
                                class="trail"
                                type="button"
                                :aria-label="reveal ? 'Hide password' : 'Show password'"
                                :aria-pressed="reveal"
                                @click="reveal = !reveal"
                            >
                                <EyeOff
                                    v-if="reveal"
                                    :size="15"
                                    :stroke-width="1.8"
                                    aria-hidden="true"
                                />
                                <Eye v-else :size="15" :stroke-width="1.8" aria-hidden="true" />
                            </button>
                        </template>
                    </SettingField>
                    <SettingField
                        v-if="mode === 'setup'"
                        id="confirm"
                        label="Password again"
                        type="password"
                        v-model="confirm"
                        autocomplete="new-password"
                    />

                    <div class="authhint">
                        <span>
                            <Lock :size="11" :stroke-width="1.9" aria-hidden="true" />
                            Session cookie · 30 days
                        </span>
                        <span v-if="username.trim()">{{ username.trim() }}</span>
                    </div>

                    <div v-if="error" ref="band" class="error-band" role="alert" tabindex="-1">
                        {{ error }}
                    </div>

                    <button
                        class="d-btn d-btn-primary submit"
                        type="submit"
                        :disabled="busy || !ready"
                    >
                        {{ mode === "setup" ? "Create the admin and sign in" : "Sign in" }}
                    </button>

                    <div class="authfoot">
                        <span class="wide-copy">Local only · no reset link</span>
                        <span class="narrow-copy">Session · 30 days</span>
                        <span class="wide-copy mono">SQLite</span>
                        <span class="narrow-copy">Local only</span>
                    </div>
                </form>
            </div>
        </div>

        <div class="signin-foot">
            <span>Personal build · no telemetry</span>
            <span class="mono">v2.0</span>
        </div>
    </section>
</template>

<style scoped>
.signin {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 540px);
    min-height: 100vh;
    min-height: 100dvh;
    background: var(--shell);
}

.signin-left {
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 0;
    padding: 46px 50px 96px;
    background: var(--shell);
    color: var(--shell-ink);
}

.signin-glow {
    position: absolute;
    right: -140px;
    top: -120px;
    width: 420px;
    height: 420px;
    border-radius: 50%;
    background: radial-gradient(circle, rgb(140 166 255 / 0.22), transparent 65%);
    pointer-events: none;
}

.authmark {
    width: 38px;
    height: 38px;
    border-radius: 11px;
    display: grid;
    place-items: center;
    flex: none;
    background: var(--brand);
    color: var(--brand-ink);
    font-family: var(--font-form);
    font-size: 15px;
    font-weight: 800;
    letter-spacing: 0.02em;
}

.signin-brand {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
}

.signin-brand b {
    font-size: 14px;
    letter-spacing: 0.01em;
}

.signin-pitch {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
}

.signin-pitch h2 {
    margin: 0;
    max-width: 24ch;
    font-family: var(--font-display);
    font-weight: 500;
    font-size: clamp(28px, 3.4vw, 44px);
    line-height: 1.12;
    letter-spacing: -0.02em;
}

.signin-pitch p {
    margin: 14px 0 0;
    max-width: 42ch;
    font-size: 12.5px;
    color: var(--shell-dim);
}

.facts {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 26px;
    margin-top: 30px;
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--shell-dim);
}

.facts b {
    display: block;
    margin-bottom: 2px;
    font-size: 12px;
    font-weight: 500;
    color: var(--shell-ink);
}

.narrow-copy {
    display: none;
}

.signin-right {
    display: grid;
    place-items: center;
    overflow-y: auto;
    padding: 40px;
    background: var(--paper);
}

.authcard {
    width: 100%;
    max-width: 360px;
}

.card-mark {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    margin-bottom: 16px;
    font-size: 13px;
}

.authcard h1 {
    margin: 0;
    font-family: var(--font-form);
    font-size: 20px;
    font-weight: 650;
    letter-spacing: -0.02em;
}

.authcard .lead {
    margin: 5px 0 22px;
    font-size: 12px;
    color: var(--ink-secondary);
}

.authcard form {
    display: grid;
    gap: 14px;
}

.authhint {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 2px 0 4px;
    font-size: 10.5px;
    color: var(--ink-tertiary);
}

.authhint span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
}

.authhint span:last-child {
    font-family: var(--font-ledger);
}

.submit {
    width: 100%;
    justify-content: center;
    margin-top: 2px;
}

.authfoot {
    display: flex;
    justify-content: space-between;
    margin-top: 4px;
    padding-top: 14px;
    border-top: 1px solid var(--rule-subtle);
    font-size: 10.5px;
    color: var(--ink-tertiary);
}

.signin-foot {
    position: absolute;
    left: 50px;
    right: calc(540px + 50px);
    bottom: 40px;
    display: flex;
    justify-content: space-between;
    font-size: 10.5px;
    color: var(--shell-dim);
}

@media (max-width: 1000px) {
    .signin {
        display: flex;
        flex-direction: column;
    }
    .signin-left {
        gap: 0;
        padding: 26px 20px 20px;
    }
    .signin-glow {
        top: -140px;
        width: 300px;
        height: 300px;
    }
    .signin-pitch h2 {
        margin-top: 14px;
        font-size: 26px;
        line-height: 1.2;
        letter-spacing: -0.01em;
    }
    .signin-pitch p {
        margin-top: 8px;
        font-size: 11px;
    }
    .facts,
    .wide-copy {
        display: none;
    }
    .narrow-copy {
        display: inline;
    }
    .signin-right {
        display: block;
        overflow: visible;
        padding: 0 14px;
        background: none;
    }
    .authcard {
        max-width: 460px;
        margin-inline: auto;
        padding: 16px;
        border-radius: var(--radius-lg);
        background: var(--paper-raised);
        box-shadow: var(--shadow-pop);
    }
    .card-mark,
    .authcard .lead {
        display: none;
    }
    .authcard h1 {
        margin-bottom: 12px;
        font-size: 16px;
    }
    .authfoot {
        margin-top: 2px;
    }
    .signin-foot {
        position: static;
        margin-top: auto;
        padding: 22px 20px;
    }
}
</style>
