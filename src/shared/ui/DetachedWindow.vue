<script setup lang="ts">
import { inject, nextTick, onUnmounted, provide, shallowRef, watch, type ShallowRef } from 'vue'
import Button from './button/Button.vue'
import { t } from '@/shared/i18n/i18n'
import { openDetachedWindow, type DetachedWindowSession } from '@/shared/window/detached-window'

const props = withDefaults(defineProps<{ open: boolean; title: string; width?: number; height?: number; busy?: boolean }>(), { width: 480, height: 320 })
const emit = defineEmits<{ close: [] }>()
// 嵌套确认框归属于当前独立窗口；业务组件和命令仍在原始 WebView 中执行。
const owner = inject<ShallowRef<Window | null> | null>('detached-window-owner', null)
const popup = shallowRef<Window | null>(null), target = shallowRef<HTMLElement | null>(null)
const failed = shallowRef(false)
provide('detached-window-owner', popup)
let session: DetachedWindowSession | null = null
function close() { if (!props.busy) emit('close') }
function dispose() {
  target.value = null; popup.value = null
  const previous = session; session = null
  // 先让 Vue 卸载 Teleport 子树，再销毁对应文档。
  if (previous) void nextTick(() => previous.dispose())
}
function open() {
  if (!props.open || session) return
  failed.value = false
  try {
    session = openDetachedWindow(owner?.value ?? window, props, () => {
      session = null; target.value = null; popup.value = null; failed.value = props.open
      close()
    }, close)
    if (session) { popup.value = session.window; target.value = session.target }
    else failed.value = true
  } catch { dispose(); failed.value = true }
}
defineExpose({ focus: () => { if (session && !session.window.closed) session.window.focus(); else open() } })
watch(() => props.open, value => { if (value) open(); else { dispose(); failed.value = false } }, { immediate: true, flush: 'sync' })
watch(() => props.title, value => { if (popup.value && !popup.value.closed) popup.value.document.title = value })
onUnmounted(dispose)
</script>

<template>
  <Teleport v-if="target" :to="target"><slot /></Teleport>
  <div v-else-if="open && failed" class="detached-window-retry" role="status"><Button variant="outline" @click="open">{{ t('打开窗口') }}</Button></div>
</template>

<style>
html.detached-window, .detached-window body { margin:0; width:100%; height:100%; overflow:hidden; background:var(--desk-bg, #fff); color:var(--desk-text, #24292f); }
.detached-window-surface { width:100%; height:100%; overflow:auto; }
.detached-window-retry { display:flex; flex-wrap:wrap; align-items:center; gap:8px; padding:8px 0; font-size:12px; }
</style>
