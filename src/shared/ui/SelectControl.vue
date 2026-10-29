<script setup lang="ts" generic="T extends string | number | null">
import { computed, ref } from 'vue'
import { handleEscape } from './escape'
import { useEscapeLayer } from './use-escape-layer'
import { Check, ChevronDown, ChevronUp } from 'lucide-vue-next'
import { SelectRoot, SelectTrigger, SelectPortal, SelectContent, SelectViewport, SelectItem, SelectItemText, SelectItemIndicator, SelectScrollUpButton, SelectScrollDownButton } from 'reka-ui'

defineOptions({ inheritAttrs: false })
const props = defineProps<{ modelValue: T; options: { value: T; label: string; disabled?: boolean }[]; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: T]; openChange: [open: boolean] }>()
const selected = computed(() => props.options.findIndex(option => option.value === props.modelValue))
const open = ref(false)
function setOpen(value: boolean) {
  if (open.value === value) return
  open.value = value; emit('openChange', value)
}
useEscapeLayer(() => open.value, () => setOpen(false))
function triggerKey(event: KeyboardEvent) {
  // 收起状态的 Esc 交给所在弹层或窗口；其他按键仍保留控件原有的隔离行为。
  if (!open.value && event.key === 'Escape') return
  event.stopPropagation()
  handleEscape(event, () => setOpen(false))
}
function menuKey(event: KeyboardEvent) { handleEscape(event, () => setOpen(false)) }
// 菜单内部只传索引，保留 0、空字符串和 null 的领域语义。
function choose(value: unknown) {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) return
  const option = props.options[Number(value)]
  if (option && !option.disabled && !props.disabled) emit('update:modelValue', option.value)
}
</script>

<template>
  <SelectRoot :open="open" :model-value="selected < 0 ? undefined : String(selected)" :disabled="disabled" @update:model-value="choose" @update:open="setOpen">
    <SelectTrigger v-bind="$attrs" class="select-trigger" @pointerdown.stop @keydown="triggerKey" @contextmenu.stop>
      <span class="select-label">{{ options[selected]?.label ?? String(modelValue ?? '') }}</span><ChevronDown :size="14" aria-hidden="true" />
    </SelectTrigger>
    <SelectPortal>
      <SelectContent class="select-menu" position="popper" :side-offset="5" :collision-padding="8" @pointerdown.stop @pointermove.stop @keydown.stop="menuKey" @contextmenu.stop>
        <SelectScrollUpButton class="select-scroll"><ChevronUp :size="14" /></SelectScrollUpButton>
        <SelectViewport class="select-viewport">
          <SelectItem v-for="(option, index) in options" :key="index" :value="String(index)" :disabled="option.disabled" class="select-item">
            <SelectItemText>{{ option.label }}</SelectItemText><SelectItemIndicator class="select-check"><Check :size="14" /></SelectItemIndicator>
          </SelectItem>
        </SelectViewport>
        <SelectScrollDownButton class="select-scroll"><ChevronDown :size="14" /></SelectScrollDownButton>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>

<style>
.select-trigger, .select-menu { --select-bg:#fff; --select-fg:#27344b; --select-border:#dfe6f1; --select-hover:#f0f5ff; --select-active:#e9f0ff; --select-accent:#3264df; }
[data-theme=dark] :is(.select-trigger,.select-menu) { --select-bg:#202938; --select-fg:#e4ebf8; --select-border:#3a4961; --select-hover:#2b3950; --select-active:#30456b; --select-accent:#abc8ff; }
.select-trigger { display:inline-flex; align-items:center; justify-content:space-between; gap:12px; min-width:100px; max-width:100%; min-height:32px; padding:5px 10px; border:1px solid var(--select-border); border-radius:6px; background:var(--select-bg); color:var(--select-fg); font:inherit; font-size:13px; text-align:left; vertical-align:middle; cursor:pointer; }
.select-trigger > svg { flex:none; color:var(--select-accent); opacity:.75; }
.select-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.select-trigger:hover:enabled { border-color:#9bb6f2; background:var(--select-hover); }
.select-trigger[data-state=open], .select-trigger:focus-visible { border-color:var(--select-accent); outline:2px solid color-mix(in srgb,var(--select-accent) 18%,transparent); outline-offset:2px; }
.select-trigger:disabled { opacity:.45; cursor:default; }
.select-menu { z-index:10000; min-width:var(--reka-select-trigger-width); max-width:min(440px,calc(100vw - 16px)); max-height:min(320px,var(--reka-select-content-available-height)); overflow:hidden; border:1px solid var(--select-border); border-radius:9px; background:var(--select-bg); color:var(--select-fg); box-shadow:0 8px 28px #152b4e1a,0 2px 6px #152b4e0a; font:13px/1.5 'Segoe UI','Microsoft YaHei',sans-serif; }
.select-viewport { padding:5px; }
.select-item { position:relative; display:flex; align-items:center; justify-content:space-between; gap:20px; min-height:32px; padding:6px 10px; border-radius:5px; outline:none; cursor:pointer; user-select:none; }
.select-item[data-state=checked] { color:var(--select-accent); background:var(--select-active); font-weight:500; }
.select-item[data-highlighted] { background:var(--select-hover); box-shadow:inset 0 0 0 1px color-mix(in srgb,var(--select-accent) 18%,transparent); }
.select-item[data-disabled] { opacity:.4; cursor:default; }
.select-check { display:flex; flex:none; color:var(--select-accent); }
.select-scroll { display:flex; align-items:center; justify-content:center; height:22px; color:var(--select-accent); }
</style>
