<script setup lang="ts">
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
import { ref, watch, type Component } from 'vue'
import { ChevronDown } from 'lucide-vue-next'
import { PopoverRoot, PopoverTrigger, PopoverPortal, PopoverContent } from 'reka-ui'
// 传入 icon 时入口只显示图标，名称和当前状态放进悬浮提示与读屏名称。
const props = defineProps<{ label: string; disabled: boolean; status?: string; active?: boolean; wide?: boolean; icon?: Component }>()
const open = ref(false)
useEscapeLayer(() => open.value, () => { open.value = false })
function dismissKey(event: KeyboardEvent) { handleEscape(event, () => { open.value = false }) }
watch(() => props.disabled, disabled => { if (disabled) open.value = false })
</script>

<template>
  <PopoverRoot v-model:open="open">
    <PopoverTrigger class="recording-option-trigger" :class="{ 'option-active': active, 'option-icon': icon }" :disabled="disabled" :aria-label="status ? `${label} · ${status}` : label" :title="icon && status ? `${label} · ${status}` : label"><component :is="icon" v-if="icon" :size="17" /><template v-else>{{ label }}<span v-if="status" class="recording-option-status">{{ status }}</span><ChevronDown :size="12" /></template></PopoverTrigger>
    <PopoverPortal>
      <!-- Portal 避免被单行工具栏裁剪；Esc 仅关闭当前选项菜单。 -->
      <PopoverContent class="recording-option-popup" :class="{ 'recording-option-popup-wide': wide }" side="bottom" align="start" :side-offset="6" :collision-padding="8" :aria-label="label" @pointerdown.stop @pointermove.stop @keydown.stop="dismissKey" @contextmenu.stop.prevent>
        <slot />
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>

<style>
.recording-option-trigger { display:inline-flex; align-items:center; gap:4px; height:28px; white-space:nowrap; }
.recording-option-trigger.option-icon { justify-content:center; min-width:30px; height:30px; padding:0 6px; border-radius:6px; }
.recording-option-status { color:#57606a; font-size:12px; font-variant-numeric:tabular-nums; }
.recording-option-trigger.option-active { color:var(--tb-active-fg, #1d4ed8); background:var(--tb-active-bg, #dbeafe); }
.option-active .recording-option-status { color:inherit; }
.recording-option-popup { z-index:2100; width:260px; max-width:calc(100vw - 16px); max-height:var(--reka-popover-content-available-height); overflow:auto; padding:10px; border:1px solid #d8dee4; border-radius:10px; background:#fff; color:#24292f; box-shadow:0 4px 16px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.recording-option-popup label { display:flex; align-items:center; gap:6px; margin:6px 0; }
.recording-option-popup-wide { width:360px; padding:14px; }
.recording-option-popup input:not([type=checkbox]):not([type=color]):not([type=range]), .recording-option-popup select { min-width:0; height:28px; padding:2px 4px; border:1px solid #d8dee4; border-radius:4px; background:inherit; color:inherit; }
.recording-option-popup small { color:#57606a; }
.recording-option-popup :focus-visible { outline:2px solid #2563eb; outline-offset:1px; }
[data-theme=dark] .recording-option-popup { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .recording-option-status { color:#a8b3c2; }
[data-theme=dark] .recording-option-trigger.option-active { color:#93c5fd; background:#253e60; }
[data-theme=dark] .option-active .recording-option-status { color:inherit; }
</style>
