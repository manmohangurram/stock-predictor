<script setup lang="ts">
/**
 * One stored exchange, opened under its row.
 *
 * The body is still produced by `call-detail.ts`: the prompt as markdown, the verdict as a card,
 * the raw bodies a click away. That module is pure — no DOM access, no listeners — so the view can
 * put it anywhere. The one behaviour it needs, copying a raw body, is delegated here.
 */
import { onMounted, ref } from "vue";
import { api, type CallDetail } from "../api";
import { renderExchangeDetail } from "./call-detail";

const props = defineProps<{ id: number }>();
const sheet = ref<HTMLElement | null>(null);
const detail = ref<CallDetail | null>(null);
const failure = ref("");

onMounted(async () => {
  try {
    detail.value = await api.callDetail(props.id);
  } catch (e) {
    failure.value = String(e);
  }
});

async function copy(btn: HTMLElement): Promise<void> {
  const source = btn.closest(".call-block")?.querySelector<HTMLElement>(".copy-src")?.textContent;
  if (!source) return;
  try {
    await navigator.clipboard.writeText(source);
    btn.dataset.copied = "true";
    btn.title = `Copied ${source.length.toLocaleString("en-IN")} characters`;
    window.setTimeout(() => {
      delete btn.dataset.copied;
      btn.title = "Copy all of it";
    }, 1400);
  } catch (e) {
    btn.title = `Could not copy: ${String(e)}`;
  }
}

function onClick(ev: MouseEvent): void {
  const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button.copy");
  if (btn) void copy(btn);
}
</script>

<template>
  <div ref="sheet" @click="onClick">
    <p v-if="failure" class="empty">Could not read call {{ id }}: {{ failure }}</p>
    <p v-else-if="!detail" class="empty">Reading the stored exchange…</p>
    <div v-else v-html="renderExchangeDetail(detail)" />
  </div>
</template>
