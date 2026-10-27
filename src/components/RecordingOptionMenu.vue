<script setup lang="ts">
import { ref, watch } from 'vue'
import { ChevronDown } from 'lucide-vue-next'
import { PopoverRoot, PopoverTrigger, PopoverPortal, PopoverContent } from 'reka-ui'
const props = defineProps<{ label: string; disabled: boolean }>()
const open = ref(false)
watch(() => props.disabled, disabled => { if (disabled) open.value = false })
</script>

<template>
  <PopoverRoot v-model:open="open">
    <PopoverTrigger class="recording-option-trigger" :disabled="disabled" :aria-label="label" :title="label">{{ label }}<ChevronDown :size="12" /></PopoverTrigger>
    <PopoverPortal>
      <!-- Portal 避免被单行工具栏裁剪；Esc 仅关闭当前选项菜单。 -->
      <PopoverContent class="recording-option-popup" side="top" align="start" :side-offset="6" :collision-padding="8" :aria-label="label" @pointerdown.stop @pointermove.stop @keydown.stop @contextmenu.stop.prevent>
        <slot />
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>

<style>
.recording-option-trigger { display:inline-flex; align-items:center; gap:4px; height:28px; white-space:nowrap; }
.recording-option-popup { z-index:2100; width:260px; max-width:calc(100vw - 16px); max-height:var(--reka-popover-content-available-height); overflow:auto; padding:10px; border:1px solid #d8dee4; border-radius:6px; background:#fff; color:#24292f; box-shadow:0 4px 16px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.recording-option-popup label { display:flex; align-items:center; gap:6px; margin:6px 0; }
.recording-option-popup input:not([type=checkbox]):not([type=color]), .recording-option-popup select { min-width:0; height:28px; padding:2px 4px; border:1px solid #d8dee4; border-radius:4px; background:inherit; color:inherit; }
.recording-option-popup small { color:#57606a; }
.recording-option-popup :focus-visible { outline:2px solid #2563eb; outline-offset:1px; }
[data-theme=dark] .recording-option-popup { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
</style>
