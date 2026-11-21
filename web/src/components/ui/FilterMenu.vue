<script setup lang="ts">
/** One audit filter: a pill that says what it is set to, and a menu of the recorded values. */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { ChevronDown } from "lucide-vue-next";
import { humanLabel } from "../../format";

const props = withDefaults(
  defineProps<{
    label: string;
    options: string[];
    modelValue?: string;
    humanise?: boolean;
  }>(),
  { modelValue: "", humanise: false },
);
const emit = defineEmits<{ "update:modelValue": [string] }>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const shown = (value: string): string =>
  props.humanise ? humanLabel(value) : value;
const current = computed(() =>
  props.modelValue ? shown(props.modelValue) : "any",
);

function pick(value: string): void {
  open.value = false;
  emit("update:modelValue", value);
}

function outside(event: MouseEvent): void {
  if (open.value && root.value && !root.value.contains(event.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener("click", outside));
onBeforeUnmount(() => document.removeEventListener("click", outside));
</script>

<template>
  <div ref="root" class="fmenu" :class="{ set: !!modelValue }">
    <button
      type="button"
      class="fbtn"
      :aria-expanded="open"
      aria-haspopup="listbox"
      :aria-label="`${label} filter, ${current}`"
      @click.stop="open = !open"
      @keydown.esc="open = false"
    >
      <span>{{ label }} · {{ current }}</span>
      <ChevronDown :size="12" :stroke-width="1.75" aria-hidden="true" />
    </button>
    <div v-if="open" class="fpop" role="listbox" :aria-label="label">
      <button
        type="button"
        role="option"
        :aria-selected="!modelValue"
        :class="{ on: !modelValue }"
        @click.stop="pick('')"
      >
        Any
      </button>
      <button
        v-for="v in options"
        :key="v"
        type="button"
        role="option"
        :aria-selected="modelValue === v"
        :class="{ on: modelValue === v }"
        @click.stop="pick(v)"
      >
        {{ shown(v) }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.fmenu {
  position: relative;
}
.fbtn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 10px;
  border: 1px solid var(--rule);
  border-radius: var(--radius-pill);
  background: var(--paper-raised);
  color: var(--ink-secondary);
  font: inherit;
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
}
.fbtn:hover {
  color: var(--ink);
  border-color: var(--rule-strong);
}
.fmenu.set .fbtn {
  color: var(--ink);
  border-color: var(--rule-strong);
}
.fbtn:focus-visible {
  outline: 2px solid var(--rule-focus);
  outline-offset: 1px;
}
.fpop {
  position: absolute;
  z-index: 30;
  top: 33px;
  left: 0;
  min-width: 180px;
  max-height: 280px;
  overflow-y: auto;
  display: grid;
  gap: 2px;
  padding: 6px;
  background: var(--paper-raised);
  border: 1px solid var(--rule);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-pop);
}
.fpop button {
  text-align: left;
  border: 0;
  background: none;
  font: inherit;
  font-size: 12px;
  color: var(--ink-secondary);
  padding: 7px 9px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.fpop button:hover,
.fpop button.on {
  background: var(--paper-inset);
  color: var(--ink);
}
.fpop button.on {
  font-weight: 600;
}
@media (max-width: 900px) {
  .fbtn {
    height: 36px;
  }
}
</style>
