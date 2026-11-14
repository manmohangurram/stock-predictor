<script setup lang="ts">
import { stamp, toneOf } from "../stamp";

/** The ledger's stamp: BUY, SELL, a status, an estimate — one shape everywhere. */
const props = withDefaults(
  defineProps<{ label: string; tone?: "buy" | "sell" | "estimate" | "blocked" }>(),
  { tone: undefined },
);
</script>

<template>
  <span v-html="stamp(props.tone ?? toneOf(props.label), props.label)" />
</template>
