<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { AppWindow, ChevronDown, Monitor, Scan } from 'lucide-vue-next'
import { DropdownMenuRoot, DropdownMenuTrigger, DropdownMenuPortal, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel } from 'reka-ui'
import { t } from '@/lib/i18n'
const props = defineProps<{ value: string; windows: { index: number; label: string; icon?: string }[]; disabled: boolean }>()
const emit = defineEmits<{ select: [value: string] }>()
const open = ref(false)
const current = computed(() => props.windows.find(item => `window:${item.index}` === props.value))
const label = computed(() => current.value?.label ?? t(props.value === 'display' ? '显示器' : props.value === 'region' ? '选区' : '窗口'))
watch(() => props.disabled, value => { if (value) open.value = false })
async function choose(value: string) {
  open.value = false
  // 先让菜单关闭、完成焦点恢复，再请求原生置前应用，避免关闭菜单时又夺回焦点。
  await nextTick()
  emit('select', value)
}
</script>

<template>
  <DropdownMenuRoot v-model:open="open">
    <DropdownMenuTrigger class="recording-source-trigger" :disabled="disabled" :aria-label="t('录制范围')" :title="label">
      <img v-if="current?.icon" :src="`data:image/png;base64,${current.icon}`" alt="" /><AppWindow v-else-if="current" :size="16" /><Monitor v-else-if="value === 'display'" :size="16" /><Scan v-else :size="16" /><span>{{ label }}</span><ChevronDown :size="12" />
    </DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent class="recording-source-menu" side="top" align="start" :side-offset="6" :collision-padding="8" :aria-label="t('录制范围')" @pointerdown.stop @pointermove.stop @keydown.stop @contextmenu.stop.prevent>
        <DropdownMenuItem class="recording-source-item" @select="choose('region')"><Scan :size="16" />{{ t('选区') }}</DropdownMenuItem>
        <DropdownMenuItem class="recording-source-item" @select="choose('display')"><Monitor :size="16" />{{ t('显示器') }}</DropdownMenuItem>
        <DropdownMenuLabel class="recording-source-caption">{{ t('应用窗口') }}</DropdownMenuLabel>
        <DropdownMenuItem v-for="item in windows" :key="item.index" class="recording-source-item" :aria-selected="value === `window:${item.index}`" @select="choose(`window:${item.index}`)"><img v-if="item.icon" :src="`data:image/png;base64,${item.icon}`" alt="" /><AppWindow v-else :size="16" /><span>{{ item.label }}</span></DropdownMenuItem>
        <DropdownMenuItem v-if="!windows.length" disabled class="recording-source-item">{{ t('没有可录制的应用窗口') }}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>

<style>
.recording-source-trigger { display:inline-flex; align-items:center; gap:4px; width:180px; max-width:180px; }
.recording-source-trigger span { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; text-align:left; white-space:nowrap; }
.recording-source-trigger img, .recording-source-menu img { width:20px; height:20px; object-fit:contain; flex:none; }
.recording-source-menu { z-index:2100; width:320px; max-width:calc(100vw - 16px); max-height:min(360px,var(--reka-dropdown-menu-content-available-height)); overflow:auto; padding:4px; border:1px solid #d8dee4; border-radius:6px; background:#fff; color:#24292f; box-shadow:0 4px 16px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.recording-source-item { display:flex; align-items:center; gap:8px; min-height:32px; padding:4px 8px; border-radius:4px; outline:none; cursor:pointer; }
.recording-source-item span { overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
.recording-source-item svg { flex:none; }
.recording-source-item[data-highlighted], .recording-source-item[aria-selected=true] { background:#eff6ff; color:#2563eb; }
.recording-source-item[data-disabled] { opacity:.5; pointer-events:none; }
.recording-source-caption { padding:6px 8px 2px; color:#57606a; font-size:12px; }
[data-theme=dark] .recording-source-menu { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .recording-source-item[data-highlighted] { background:#303f56; color:#93c5fd; }
</style>
