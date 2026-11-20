<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, auth } from "../api";

const router = useRouter();
const mode = ref<"login" | "setup">("login");
const username = ref("admin");
const password = ref("");
const confirm = ref("");
const error = ref("");
const busy = ref(false);
const ready = ref(false);

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

async function submit(): Promise<void> {
  error.value = "";
  if (mode.value === "setup" && password.value !== confirm.value) {
    error.value = "The two passwords do not match.";
    return;
  }
  busy.value = true;
  try {
    const body = { username: username.value.trim(), password: password.value };
    const res = mode.value === "setup" ? await api.authSetup(body) : await api.authLogin(body);
    auth.set(res.access_token);
    await router.push("/");
  } catch (e) {
    const text = String(e);
    error.value = text.includes("401")
      ? "That password does not match."
      : text.includes("409")
        ? "This instance already has an admin. Sign in instead."
        : text;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="login">
    <div class="card">
      <header>
        <h1>{{ mode === "setup" ? "Set up the admin" : "Sign in" }}</h1>
        <p class="lede">
          {{
            mode === "setup"
              ? "This book has no password yet. Choose one now — it is the only door, and it cannot be recovered."
              : "One admin, one password. Your book is behind it."
          }}
        </p>
      </header>

      <form autocomplete="off" @submit.prevent="submit">
        <label class="field">
          <span class="lbl">Username</span>
          <input v-model="username" type="text" autocapitalize="none" :readonly="!ready" />
        </label>
        <label class="field">
          <span class="lbl">Password</span>
          <input
            v-model="password"
            type="password"
            :placeholder="mode === 'setup' ? 'at least 10 characters' : ''"
            autofocus
          />
        </label>
        <label v-if="mode === 'setup'" class="field">
          <span class="lbl">Again</span>
          <input v-model="confirm" type="password" />
        </label>

        <div v-if="error" class="error-band" role="alert">{{ error }}</div>

        <button class="d-btn d-btn-primary wide" type="submit" :disabled="busy || !ready">
          {{ mode === "setup" ? "Create the admin and sign in" : "Sign in" }}
        </button>
      </form>
    </div>
  </section>
</template>

<style scoped>
.login {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: var(--space-6);
  background: var(--paper);
}
.card {
  width: min(420px, 100%);
  background: var(--paper-raised);
  border: 1px solid var(--rule-strong);
  border-radius: var(--radius);
  padding: var(--space-6);
  box-shadow: var(--shadow-1);
}
h1 { font-family: var(--font-form); font-size: var(--text-xl); margin: 0 0 var(--space-2); }
.lede { color: var(--ink-muted); margin: 0 0 var(--space-5); }
form { display: grid; gap: var(--space-4); }
.wide { width: 100%; }
</style>
