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
  <div class="max-w-72 text-right">
    <button ref="button" type="button" :disabled="disabled && !editing && !busy" :aria-label="t('{0}快捷键，点击修改', { 0: t(label) })" :aria-pressed="editing" class="min-w-40 rounded-lg border px-3 py-2 text-sm focus:outline-blue-500" :class="editing ? 'border-blue-500 bg-blue-50' : 'border-zinc-200 bg-white'" @click="begin" @keydown="keydown" @keyup="keyup" @blur="finish()">
      {{ busy ? t("正在切换…") : editing ? preview : modelValue || t("已禁用 · 点击设置") }}
    </button>
    <p v-if="editing" class="mt-1 text-xs text-zinc-500">{{ t("Esc 取消 · Backspace / Delete 禁用") }}</p>
    <p v-if="error" class="mt-1 text-xs text-red-600" role="alert">{{ te(error) }}</p>
  </div>
</template>
