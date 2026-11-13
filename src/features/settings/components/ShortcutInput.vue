<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { api } from '@/services/api'
import { captureShortcut } from '@/features/settings/settings'

const props = defineProps<{ modelValue: string; disabled?: boolean; label: string; validate: (value: string) => string | null }>()
const emit = defineEmits<{ 'update:modelValue': [value: string]; editing: [value: boolean]; error: [message: string] }>()
const editing = ref(false), busy = ref(false), preview = ref(''), error = ref('')
const button = ref<HTMLButtonElement>()
let disposed = false
let canceled = false
let candidate: { value: string; code: string; released: boolean } | null = null

/** 注销成功后才捕获；等待时失焦也要恢复。 */
async function begin() {
  if (props.disabled || busy.value || editing.value) return
  busy.value = true; canceled = false; candidate = null; error.value = ''; emit('editing', true)
  try {
    await api.setShortcutEditing(true)
    if (disposed || canceled || !document.hasFocus() || document.activeElement !== button.value) {
      await api.setShortcutEditing(false)
      emit('editing', false)
      return
    }
    editing.value = true; preview.value = '请按快捷键…'
  } catch (e) { emit('error', String(e)); emit('editing', false) }
  finally { busy.value = false }
}
/** 先恢复原映射，再更新草稿；保存失败仍保留已生效配置。 */
async function finish(value?: string) {
  canceled = true
  if (!editing.value || busy.value) return
  editing.value = false; busy.value = true
  try {
    await api.setShortcutEditing(false)
    if (!disposed && value !== undefined) emit('update:modelValue', value)
  } catch (e) { emit('error', `快捷键恢复失败，请重新保存设置或重启应用：${String(e)}`) }
  finally { busy.value = false; emit('editing', false) }
}
function keydown(event: KeyboardEvent) {
  if (!editing.value) return
  event.preventDefault(); event.stopPropagation()
  if (event.repeat || event.isComposing) return
  const result = captureShortcut(event)
  if (result.cancel) { void finish(); return }
  if (result.error || result.value !== undefined) candidate = null
  error.value = result.error ?? ''
  if (result.preview !== undefined) preview.value = result.preview || '请按快捷键…'
  if (result.value !== undefined) {
    error.value = props.validate(result.value) ?? ''
    if (!error.value) {
      candidate = { value: result.value, code: event.code, released: false }
      preview.value = result.value || '已禁用'
    }
  }
}
/** 等主键和修饰键全部松开再恢复，避免把仍按住的 F1 当成新的全局触发。 */
function keyup(event: KeyboardEvent) {
  if (!editing.value) return
  event.preventDefault(); event.stopPropagation()
  if (!candidate) return
  if (event.code === candidate.code) candidate.released = true
  if (candidate.released && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) void finish(candidate.value)
}
function cancel() { void finish() }
onMounted(() => window.addEventListener('blur', cancel))
onBeforeUnmount(() => { window.removeEventListener('blur', cancel); disposed = true; void finish() })
</script>

<template>
  <div class="shortcut-input">
    <button ref="button" type="button" class="shortcut-field" :class="{ editing }" :disabled="disabled && !editing && !busy" :aria-label="t('{0}快捷键，点击修改', { 0: t(label) })" :aria-pressed="editing" @click="begin" @keydown="keydown" @keyup="keyup" @blur="finish()">
      <span v-if="busy" class="shortcut-placeholder">{{ t("正在切换…") }}</span>
      <span v-else-if="editing" class="shortcut-placeholder">{{ t(preview) }}</span>
      <template v-else-if="modelValue"><kbd v-for="key in modelValue.split('+')" :key="key">{{ key }}</kbd></template>
      <span v-else class="shortcut-placeholder">{{ t("未设置") }}</span>
    </button>
    <p v-if="editing" class="shortcut-help">{{ t("单按 Esc 取消 · 单按 Backspace / Delete 禁用") }}</p>
    <p v-if="error" class="shortcut-help shortcut-error" role="alert">{{ te(error) }}</p>
  </div>
</template>

<style scoped>
.shortcut-input { display:flex; flex-direction:column; align-items:flex-end; }
.shortcut-field { display:inline-flex; align-items:center; gap:3px; min-width:120px; height:26px; padding:0 4px; border:1px solid var(--desk-line,#d3d8de); border-radius:6px; background:var(--desk-surface,#fff); cursor:pointer; }
.shortcut-field:hover:enabled { border-color:var(--desk-muted,#5f6670); }
.shortcut-field.editing { border-color:var(--desk-accent,#3b5bdb); background:var(--desk-soft,#eef1fc); }
.shortcut-field:disabled { opacity:.45; cursor:default; }
.shortcut-field kbd { height:18px; min-width:18px; padding:0 5px; }
.shortcut-placeholder { padding:0 4px; font-size:12px; color:var(--desk-muted,#5f6670); white-space:nowrap; }
.shortcut-help { margin:4px 0 0; font-size:11px; color:var(--desk-muted,#5f6670); }
.shortcut-error { color:var(--desk-danger,#cf222e); }
</style>
