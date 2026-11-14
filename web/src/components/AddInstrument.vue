<script setup lang="ts">
/**
 * Adding an instrument: one dialog, a search you pick from, results grouped by class, and the
 * keyboard doing the rest. ⌘K / Ctrl-K opens it from anywhere.
 *
 * Only Indian listings appear: the book is one currency and one exchange's rules, so a NYSE ADR
 * would be priced in dollars and read here as rupees.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type Hit } from "../api";

const router = useRouter();
const open = ref(false);
const query = ref("");
const hits = ref<Hit[]>([]);
const active = ref(0);
const busy = ref(false);
const failure = ref("");
const dialog = ref<HTMLDialogElement | null>(null);
const input = ref<HTMLInputElement | null>(null);
let timer: number | undefined;
let seq = 0;

const SHOWN = 6;

const indian = (h: Hit): boolean =>
  h.class === "mutual_fund" || h.venue === "NSE" || h.venue === "Bombay" || /\.(NS|BO)$/.test(h.code);

const groups = computed(() => {
  const local = hits.value.filter(indian);
  return [
    { label: "Stocks", items: local.filter((h) => h.class !== "mutual_fund") },
    { label: "Mutual funds", items: local.filter((h) => h.class === "mutual_fund") },
  ].filter((g) => g.items.length);
});

const flat = computed(() => groups.value.flatMap((g) => g.items));

function show(): void {
  open.value = true;
  query.value = "";
  hits.value = [];
  active.value = 0;
  failure.value = "";
  dialog.value?.showModal();
  requestAnimationFrame(() => input.value?.focus());
}

function close(): void {
  open.value = false;
  dialog.value?.close();
}

async function search(): Promise<void> {
  const q = query.value.trim();
  if (q.length < 2) {
    hits.value = [];
    return;
  }
  const mine = ++seq;
  try {
    const found = await api.search(q);
    if (mine === seq) {
      hits.value = found;
      active.value = 0;
    }
  } catch {
    if (mine === seq) hits.value = [];
  }
}

function onInput(): void {
  window.clearTimeout(timer);
  timer = window.setTimeout(() => void search(), 220);
}

async function add(hit: Hit | undefined): Promise<void> {
  if (!hit || busy.value) return;
  busy.value = true;
  failure.value = "";
  try {
    await api.addInstrument(hit.code, hit.name, hit.class);
    close();
    await router.push(`/i/${encodeURIComponent(hit.code)}`);
  } catch (e) {
    failure.value = String(e);
  } finally {
    busy.value = false;
  }
}

function key(ev: KeyboardEvent): void {
  if (ev.key === "ArrowDown") {
    ev.preventDefault();
    active.value = Math.min(active.value + 1, Math.min(flat.value.length, SHOWN) - 1);
  } else if (ev.key === "ArrowUp") {
    ev.preventDefault();
    active.value = Math.max(active.value - 1, 0);
  } else if (ev.key === "Enter") {
    ev.preventDefault();
    void add(flat.value[active.value]);
  }
}

function onKeydown(ev: KeyboardEvent): void {
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
    ev.preventDefault();
    show();
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <button class="d-btn d-btn-sm d-btn-primary head-action" type="button" aria-haspopup="dialog" @click="show">Add instrument…</button>
  <dialog ref="dialog" class="add-dialog" @close="open = false">
    <div class="add-dialog-body">
      <div class="dlg-search">
        <input
          ref="input"
          type="search"
          autocomplete="off"
          aria-label="Search stocks and mutual funds"
          placeholder="Infosys, INFY, 120503, Motilal Oswal Midcap…"
          role="combobox"
          aria-expanded="true"
          aria-controls="results"
          :value="query"
          @input="query = ($event.target as HTMLInputElement).value; onInput()"
          @keydown="key"
        />
        <span class="kbd">esc</span>
      </div>
      <div id="results" class="results dlg-results" role="listbox" aria-label="Search results">
        <template v-if="flat.length">
          <template v-for="g in groups" :key="g.label">
            <div class="dlg-group">{{ g.label }}</div>
            <div
              v-for="h in g.items"
              :key="h.code"
              class="dlg-opt"
              :class="{ sel: flat[active]?.code === h.code }"
              role="option"
              :aria-selected="flat[active]?.code === h.code"
              tabindex="-1"
              @click="add(h)"
            >
              <b>{{ h.name }}</b>
              <span class="meta">{{ h.code }} · {{ h.class === "mutual_fund" ? "fund" : "stock" }}</span>
            </div>
          </template>
        </template>
        <p v-else class="empty">
          {{ query.trim().length >= 2 ? "Nothing matches that." : "Type a name, a ticker, or an AMFI code." }}
        </p>
      </div>
      <p v-if="failure" class="help" role="status">{{ failure }}</p>
      <div class="dlg-foot"><span>↑↓ to move</span><span>⏎ to add</span><span>Esc to close</span></div>
    </div>
  </dialog>
</template>
