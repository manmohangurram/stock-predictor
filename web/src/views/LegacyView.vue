<script setup lang="ts">
/**
 * Stage 1 of the Vue migration: the stack and the shell are Vue, the pages are still the plain
 * renderers. One route renders here, the existing wiring is attached after the markup lands, and
 * the next stage replaces whichever page it names with a real view.
 *
 * This component exists to be deleted. It is the seam that lets every other page keep working while
 * one page at a time moves out of it.
 */
import { onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { draw } from "../renderer";

const el = ref<HTMLElement | null>(null);
const route = useRoute();
const router = useRouter();

async function paint(): Promise<void> {
  if (el.value) await draw(el.value, route.fullPath);
}

// The router resolves the URL asynchronously; painting before that draws the wrong page.
onMounted(async () => {
  await router.isReady();
  await paint();
});
watch(() => route.fullPath, paint);
</script>

<template>
  <div ref="el" />
</template>
