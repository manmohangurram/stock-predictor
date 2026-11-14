<script setup lang="ts">
/** An empty state: one sentence about what is absent, never a shrug. */
defineProps<{ text: string }>();
</script>

<template>
  <p class="empty">{{ text }}</p>
</template>
