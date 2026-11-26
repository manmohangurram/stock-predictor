<script setup lang="ts">
/**
 * One stored exchange, opened under its row: the prompt as prose and the reply as a parsed answer.
 *
 * The body is produced by `call-detail.ts` — pure functions, no DOM — so the same view can sit in
 * the desktop row and in the phone card. Copying a raw body is the one behaviour the view needs.
 */
import { onMounted, ref } from "vue";
import { api, type CallDetail } from "../api";
import { renderParsed } from "./call-detail";

const props = withDefaults(defineProps<{ id: string; compact?: boolean }>(), {
    compact: false,
});
const detail = ref<CallDetail | null>(null);
const failure = ref("");

onMounted(async () => {
    try {
        detail.value = await api.callDetail(props.id);
    } catch (e) {
        failure.value = String(e);
    }
});

async function onClick(event: MouseEvent): Promise<void> {
    const btn = (event.target as HTMLElement).closest<HTMLButtonElement>("button.ccopy");
    if (!btn) return;
    const text = btn.closest(".crawcol")?.querySelector("pre")?.textContent ?? "";
    try {
        await navigator.clipboard.writeText(text);
        btn.textContent = "Copied";
        window.setTimeout(() => (btn.textContent = "Copy"), 1400);
    } catch {
        btn.textContent = "Could not copy";
    }
}
</script>

<template>
    <div class="call-sheet" @click="onClick">
        <p v-if="failure" class="empty">Could not read call {{ id }}: {{ failure }}</p>
        <p v-else-if="!detail" class="empty">Reading the stored exchange…</p>
        <div v-else v-html="renderParsed(detail as never, compact)" />
    </div>
</template>
