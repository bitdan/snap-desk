<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Check, ChevronDown, Monitor, Scan } from 'lucide-vue-next'
import { DropdownMenuRoot, DropdownMenuTrigger, DropdownMenuPortal, DropdownMenuContent, DropdownMenuItem } from 'reka-ui'
import { t } from '@/shared/i18n/i18n'
const props = defineProps<{ value: 'region' | 'display'; disabled: boolean }>()
const emit = defineEmits<{ select: [value: string] }>()
const open = ref(false)
const label = computed(() => t(props.value === 'display' ? '显示器' : '选区'))
watch(() => props.disabled, value => { if (value) open.value = false })
async function choose(value: string) {
  open.value = false
  // 先关闭菜单并恢复焦点，再更新选区，避免键盘焦点留在已卸载的菜单项上。
  await nextTick()
  emit('select', value)
}
</script>

<template>
  <DropdownMenuRoot v-model:open="open">
    <DropdownMenuTrigger class="recording-source-trigger" :disabled="disabled" :aria-label="t('录制范围')" :title="label">
      <Monitor v-if="value === 'display'" :size="16" /><Scan v-else :size="16" /><span>{{ label }}</span><ChevronDown :size="12" />
    </DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent class="recording-source-menu" side="top" align="start" :side-offset="6" :collision-padding="8" :aria-label="t('录制范围')" @pointerdown.stop @pointermove.stop @keydown.stop @contextmenu.stop.prevent>
        <DropdownMenuItem class="recording-source-item" :aria-selected="value === 'region'" @select="choose('region')"><Scan :size="16" /><span>{{ t('选区') }}</span><Check v-if="value === 'region'" :size="14" class="source-check" /></DropdownMenuItem>
        <DropdownMenuItem class="recording-source-item" :aria-selected="value === 'display'" @select="choose('display')"><Monitor :size="16" /><span>{{ t('显示器') }}</span><Check v-if="value === 'display'" :size="14" class="source-check" /></DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>

<style>
.recording-source-trigger { display:inline-flex; align-items:center; gap:4px; width:auto; min-width:84px; max-width:128px; }
.recording-source-trigger span { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; text-align:left; white-space:nowrap; }
.recording-source-menu { z-index:2100; width:148px; max-width:calc(100vw - 16px); max-height:min(360px,var(--reka-dropdown-menu-content-available-height)); overflow:auto; padding:4px; border:1px solid #d8dee4; border-radius:6px; background:#fff; color:#24292f; box-shadow:0 4px 16px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.recording-source-item { display:flex; align-items:center; gap:8px; min-height:32px; padding:4px 8px; border-radius:4px; outline:none; cursor:pointer; }
.recording-source-item span { flex:1; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
.recording-source-item svg { flex:none; }
.recording-source-item[data-highlighted], .recording-source-item[aria-selected=true] { background:#eff6ff; color:#2563eb; }
.recording-source-item[data-disabled] { opacity:.5; pointer-events:none; }
.source-check { margin-left:auto; color:var(--ui-success-fg); }
[data-theme=dark] .recording-source-menu { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .recording-source-item[data-highlighted] { background:#303f56; color:#93c5fd; }
</style>
