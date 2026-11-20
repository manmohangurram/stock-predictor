<script setup lang="ts">
import DangerZone from "../components/DangerZone.vue";

const symbols = ["INFY.NS", "RELIANCE.NS", "TCS.NS"];
</script>

<template>
  <section class="preview">
    <h1>Delete component — pick one</h1>
    <p class="lede">
      Same behaviour in all three: the confirmation is a symbol from the book. They differ in how
      much ceremony they put in front of it.
    </p>

    <div class="option">
      <span class="tag">Option A — one line, symbol locked</span>
      <DangerZone variant="A" :symbols="symbols" />
    </div>

    <div class="option">
      <span class="tag">Option B — two steps, nothing revealed until asked</span>
      <DangerZone variant="B" :symbols="symbols" />
    </div>

    <div class="option">
      <span class="tag">Option C — show the damage first, button unlocks on the right symbol</span>
      <DangerZone variant="C" :symbols="symbols" />
    </div>
  </section>
</template>

<style scoped>
.preview { display: grid; gap: var(--space-4); max-width: 900px; }
h1 { font-family: var(--font-form); font-size: var(--text-2xl); margin: 0; }
.lede { margin: 0 0 var(--space-2); color: var(--ink-secondary); max-width: 68ch; }
.option { display: grid; gap: var(--space-2); }
.tag {
  justify-self: start;
  font-family: var(--font-form);
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: .06em;
  text-transform: uppercase;
  color: var(--ink-tertiary);
}
</style>
