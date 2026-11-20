<script setup lang="ts">
/** One labelled control, with the help line that says what it does to the money. */
withDefaults(
  defineProps<{
    id: string;
    label: string;
    unit?: string;
    help?: string;
    type?: string;
    step?: string;
    placeholder?: string;
    min?: string;
    disabled?: boolean;
    autocomplete?: string;
    autofocus?: boolean;
    modelValue: string;
  }>(),
  {
    type: "text",
    unit: "",
    help: "",
    step: undefined,
    placeholder: "",
    min: undefined,
    disabled: false,
    autocomplete: "off",
    autofocus: false,
  },
);
defineEmits<{ "update:modelValue": [string] }>();
</script>

<template>
  <div class="field">
    <label class="lbl" :for="id">{{ label }}</label>
    <div class="field-line">
      <input
        :id="id"
        :name="id"
        :type="type"
        :step="step"
        :min="min"
        :placeholder="placeholder"
        :disabled="disabled"
        :autocomplete="autocomplete"
        :autofocus="autofocus"
        :value="modelValue"
        @input="$emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      />
      <span v-if="unit" class="unit">{{ unit }}</span>
    </div>
    <p v-if="help" class="help" v-html="help" />
  </div>
</template>
