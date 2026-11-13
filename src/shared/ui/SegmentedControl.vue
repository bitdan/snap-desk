<script setup lang="ts" generic="T extends string | number">
/** 少量互斥选项的分段控件；选中项用 aria-pressed 表达，外观见 workspace.css 的 .segmented-control。 */
defineProps<{ modelValue: T; options: { value: T; label: string; disabled?: boolean }[]; disabled?: boolean; label?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: T] }>()
</script>

<template>
  <div class="segmented-control" role="group" :aria-label="label">
    <button v-for="option in options" :key="String(option.value)" type="button" :aria-pressed="modelValue === option.value" :disabled="disabled || option.disabled" @click="modelValue !== option.value && emit('update:modelValue', option.value)">{{ option.label }}</button>
  </div>
</template>
