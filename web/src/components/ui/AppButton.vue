<script setup lang="ts">
/** A button with the app's one button look: `.btn`, plus `primary` and `ghost` variants. */
withDefaults(
  defineProps<{
    variant?: "plain" | "primary" | "ghost";
    type?: "button" | "submit";
    disabled?: boolean;
  }>(),
  { variant: "plain", type: "button", disabled: false },
);
</script>

<template>
  <button :type="type" :disabled="disabled" :class="['btn', variant === 'plain' ? '' : variant]">
    <slot />
  </button>
</template>
