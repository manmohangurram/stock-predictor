<script setup lang="ts">
/**
 * What left the book, and what its fills actually made.
 *
 * There is no price left to value an archived instrument at, so the profit comes from the fills
 * themselves — realised on the sells, and money in minus money out for the round trip. The page
 * lists only instruments a person archived; a name the scan merely noticed is not here.
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type ArchivedInstrument } from "../api";
import { histDate, money } from "../format";
import SettingField from "../components/SettingField.vue";

const router = useRouter();
const rows = ref<ArchivedInstrument[]>([]);
const error = ref("");

onMounted(async () => {
  try {
    rows.value = await api.archived();
  } catch (e) {
    error.value = String(e);
  }
});

const phrase = ref("");
const armed = ref(false);
const busy = ref(false);
const resetNote = ref("");
const resetError = ref("");

async function wipe(): Promise<void> {
  resetError.value = "";
  resetNote.value = "";
  if (phrase.value.trim() !== "DELETE EVERYTHING") {
    resetError.value = 'Type DELETE EVERYTHING, exactly, to confirm.';
    return;
  }
  if (!armed.value) {
    armed.value = true;
    return;
  }
  busy.value = true;
  try {
    const res = await api.resetBook(phrase.value.trim());
    const count = Object.values(res.deleted).reduce((n, v) => n + v, 0);
    resetNote.value = `Deleted ${count} rows. The book is empty; you are still signed in.`;
    phrase.value = "";
    armed.value = false;
    rows.value = [];
  } catch (e) {
    resetError.value = String(e);
  } finally {
    busy.value = false;
  }
}

const total = computed(() => rows.value.reduce((n, r) => n + r.realised, 0));
const fills = computed(() => rows.value.reduce((n, r) => n + r.fills, 0));

/** An open round trip has nothing realised yet: an em dash, not a confident zero. */
function realised(r: ArchivedInstrument): { text: string; tone: string } {
  const nothing = Math.abs(r.realised) < 0.0005 && r.net_invested > 0.0005;
  if (r.fills === 0 || nothing) return { text: "—", tone: "" };
  return {
    text: `${r.realised >= 0 ? "+" : "−"}${money(Math.abs(r.realised))}`,
    tone: r.realised > 0.0005 ? "up" : r.realised < -0.0005 ? "down" : "",
  };
}

const open = (r: ArchivedInstrument): string =>
  Math.abs(r.net_invested) > 0.0005
    ? `${money(Math.abs(r.net_invested))} ${r.net_invested > 0 ? "in" : "out"}`
    : "closed out";

const visit = (symbol: string): void => void router.push(`/i/${encodeURIComponent(symbol)}`);
</script>

<template>
  <div v-if="error" class="error-band">Something failed: {{ error }}</div>
  <template v-else>
    <div class="page-head">
      <div>
        <h1>Archived</h1>
        <div class="page-sub">What left the book, and what it made — history without the prices</div>
      </div>
      <div class="head-right">
        <div class="head-row">
          <div class="figures">
            <div><span class="lbl">Instruments</span><span class="fig-line"><b class="num">{{ rows.length }}</b></span></div>
            <div><span class="lbl">Fills kept</span><span class="fig-line"><b class="num">{{ fills }}</b></span></div>
            <div><span class="lbl">Realised</span><span class="fig-line">
              <b class="num" :class="total === 0 ? '' : total > 0 ? 'pos' : 'neg'">{{
                total === 0 ? money(0, 0) : `${total > 0 ? "+" : "−"}${money(Math.abs(total), 0)}`
              }}</b></span></div>
          </div>
        </div>
      </div>
    </div>

    <section class="sect">
      <div class="table-wrap">
        <table class="blotter">
          <thead><tr>
            <th scope="col">Holding</th><th scope="col" class="hide-sm num">Fills</th>
            <th scope="col" class="hide-sm num">Decisions</th>
            <th scope="col" class="hide-sm num">Last fill</th>
            <th scope="col" class="num">Realised</th>
          </tr></thead>
          <tbody>
            <tr
              v-for="r in rows"
              :key="r.id"
              class="row"
              tabindex="0"
              role="link"
              :aria-label="`${r.name}, archived — open its record`"
              @click="visit(r.symbol)"
              @keydown.enter="visit(r.symbol)"
            >
              <td class="holding">
                <div class="name">{{ r.name }}</div>
                <div class="sub">{{ r.symbol }} · {{ r.class.replace("_", " ") }}</div>
              </td>
              <td class="num hide-sm">{{ r.fills }}</td>
              <td class="num hide-sm">{{ r.decisions }}</td>
              <td class="num hide-sm">{{ r.last_fill ? histDate(r.last_fill) : "—" }}</td>
              <td class="num" :class="realised(r).tone">
                {{ realised(r).text }}
                <div class="sub">{{ open(r) }}</div>
              </td>
            </tr>
            <tr v-if="!rows.length">
              <td colspan="5" class="empty">
                Nothing archived yet. Archive an instrument from its page — its record lands here.
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <section class="danger">
      <header class="pane-head">
        <h2>Delete everything</h2>
        <span class="pill todo">cannot be undone</span>
      </header>
      <p class="lede">
        This clears the whole book: instruments, bars, holdings, fills, the cash ledger, decisions,
        notifications, runs, call records, MCP keys and settings. The admin account and this session
        stay, so you land on an empty book rather than the setup screen. There is no backup here —
        copy the database file first if you might want any of it.
      </p>
      <form class="trade-form" autocomplete="off" @submit.prevent="wipe">
        <div class="row">
          <SettingField id="reset_phrase" label='Type DELETE EVERYTHING' v-model="phrase"
            placeholder="DELETE EVERYTHING" autocomplete="off" />
          <button class="d-btn d-btn-sm" type="submit" :disabled="busy" :class="{ danger: armed }">
            {{ armed ? "Click again to delete everything" : "Delete everything" }}
          </button>
        </div>
        <div v-if="resetError" class="error-band" role="alert">{{ resetError }}</div>
        <p v-if="resetNote" class="note" role="status">{{ resetNote }}</p>
      </form>
    </section>
  </template>
</template>
<style scoped>
.danger { margin-top: var(--space-8); border-top: 1px solid var(--rule); padding-top: var(--space-4); }
.danger h2 { color: var(--stamp-oxide); }
.danger .pill { border-radius: var(--radius-pill); }

.d-btn.danger { color: var(--stamp-oxide); border-color: var(--stamp-oxide); }
</style>
