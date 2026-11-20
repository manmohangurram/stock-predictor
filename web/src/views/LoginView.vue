<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, auth } from "../api";
import SettingField from "../components/SettingField.vue";

const router = useRouter();
const mode = ref<"login" | "setup">("login");
const username = ref("admin");
const password = ref("");
const confirm = ref("");
const error = ref("");
const busy = ref(false);
const ready = ref(false);
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
    const body = { username: username.value.trim(), password: password.value };
    const res = mode.value === "setup" ? await api.authSetup(body) : await api.authLogin(body);
    auth.set(res.access_token);
    await router.push("/");
  } catch (e) {
    const text = String(e);
    if (text.includes("401")) await fail("That password does not match.");
    else if (text.includes("409")) await fail("This instance already has an admin. Sign in instead.");
    else await fail(text);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="gate">
    <div class="gate-card">
      <div class="gate-brand">
        <img src="/favicon.svg" width="28" height="28" alt="" aria-hidden="true" />
        <b>Stock Predictor</b>
      </div>

      <header class="pane-head">
        <h1>{{ mode === "setup" ? "Set up the admin" : "Sign in" }}</h1>
      </header>
      <p class="lede">
        {{
          mode === "setup"
            ? "This book has no password yet. Choose one now — it is the only door, and it cannot be recovered."
            : "One admin, one password. Your book is behind it."
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
          type="password"
          v-model="password"
          :autocomplete="mode === 'setup' ? 'new-password' : 'current-password'"
          :placeholder="mode === 'setup' ? '10+ characters' : ''"
          autofocus
        />
        <SettingField
          v-if="mode === 'setup'"
          id="confirm"
          label="Password again"
          type="password"
          v-model="confirm"
          autocomplete="new-password"
        />

        <div v-if="error" ref="band" class="error-band" role="alert" tabindex="-1">{{ error }}</div>

        <div class="gate-foot">
          <button class="d-btn d-btn-primary" type="submit" :disabled="busy || !ready">
            {{ mode === "setup" ? "Create the admin and sign in" : "Sign in" }}
          </button>
          <span class="note">{{ mode === "setup" ? "First run — this happens once." : "" }}</span>
        </div>
      </form>
    </div>
  </section>
</template>

<style scoped>
/*
 * One left edge and one right edge for everything in the card: the brand, the title, the lede, the
 * labels, the inputs, the button and the note all share the content width. The fields are allowed
 * past the 232px cap the settings panes use, because a card has only one column to fill.
 */
.gate {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: var(--space-6) var(--space-4);
  background: var(--paper);
}
.gate-card,
.gate-card :deep(*) { box-sizing: border-box; }
.gate-card {
  width: min(440px, 100%);
  background: var(--paper-raised);
  border: 1px solid var(--rule-strong);
  border-radius: var(--radius);
  padding: var(--space-6);
  box-shadow: var(--shadow-1);
}
.gate-brand {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0 0 var(--space-4);
  font-family: var(--font-form);
}
.gate-brand b { font-size: var(--text-base); }
.pane-head { margin: 0; }
h1 {
  font-family: var(--font-form);
  font-size: var(--text-xl);
  line-height: 1.2;
  margin: 0 0 var(--space-2);
}
.lede { color: var(--ink-muted); margin: 0 0 var(--space-6); }
form { display: grid; gap: var(--space-4); }
form :deep(.field) { width: 100%; }
form :deep(.field input) { max-width: none; width: 100%; min-width: 0; }
.error-band { margin: 0; }
.gate-foot {
  display: grid;
  gap: var(--space-3);
  margin-top: var(--space-1);
}
.gate-foot .d-btn { width: 100%; }
.gate-foot .note { text-align: start; margin: 0; }
</style>
