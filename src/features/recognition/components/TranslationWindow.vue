<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { Camera, Pin, X } from 'lucide-vue-next'
import { api, onTranslationOpen } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import { handleWindowEscape } from '@/shared/ui/escape'
import Button from '@/shared/ui/button/Button.vue'
import TranslationPanel from './TranslationPanel.vue'

const text = ref(''), error = ref('')
const revision = ref(0), ready = ref(false), capturing = ref(false), automatic = ref(false)
const translating = ref(false)
const pinned = ref(false), controlling = ref(false)
let disposed = false, off: (() => void) | undefined
let reading: Promise<void> = Promise.resolve()
/** 先订阅再补读；领取串行化，避免冷启动与重复通知把新原文覆盖为旧原文。 */
function receive() {
  reading = reading.catch(() => {}).then(async () => {
    const pending = await api.readTranslationInput('pending')
    if (disposed) return
    if (pending !== null) {
      ready.value = false
      await nextTick()
      translating.value = false
      text.value = pending; automatic.value = true; revision.value++
    }
    ready.value = true
  }).catch(e => { if (!disposed) { error.value = String(e); ready.value = true } })
  return reading
}
async function close() {
  try { await api.closeTranslation() } catch (e) { error.value = String(e) }
}
/** 无系统标题栏时，拖动和置顶仍由受窗口权限限制的原生命令完成。 */
async function control(action: 'drag' | 'pin' | 'unpin') {
  if (controlling.value) return
  controlling.value = true; error.value = ''
  try {
    await api.controlTranslationWindow(action)
    if (action !== 'drag') pinned.value = action === 'pin'
  } catch (e) { error.value = String(e) }
  finally { controlling.value = false }
}
async function capture() {
  if (capturing.value || translating.value) return
  capturing.value = true; error.value = ''
  try { await api.startTranslationCapture() }
  catch (e) { error.value = String(e) }
  finally { capturing.value = false }
}
async function initialize() {
  try {
    const unsubscribe = await onTranslationOpen(() => { void receive() })
    if (disposed) { unsubscribe(); return }
    off = unsubscribe
    await receive()
  } catch (e) { if (!disposed) error.value = String(e) }
}
onMounted(() => { void initialize() })
onUnmounted(() => { disposed = true; off?.() })
</script>

<template>
  <main class="translation-window" :aria-label="t('文字翻译')" @keydown="handleWindowEscape($event, close)">
    <header class="translation-tools">
      <Button variant="ghost" :disabled="controlling" :aria-pressed="pinned" :aria-label="pinned ? t('取消置顶') : t('置顶窗口')" :title="pinned ? t('取消置顶') : t('置顶窗口')" @click="control(pinned ? 'unpin' : 'pin')"><Pin :size="17" :fill="pinned ? 'currentColor' : 'none'" /></Button>
      <div class="window-drag" @pointerdown="$event.button === 0 && control('drag')" />
      <Button variant="ghost" :aria-label="t('关闭翻译')" :title="t('关闭（Esc）')" @click="close"><X :size="16" /></Button>
    </header>
    <p v-if="error" class="window-error" role="alert">{{ te(error) }}</p>
    <TranslationPanel v-if="ready" :key="revision" v-model:text="text" workspace standalone :auto-translate="automatic" @busy="translating = $event" @close="close">
      <template #source-actions><Button class="source-action" variant="ghost" :disabled="capturing || translating" :title="t('截图翻译')" :aria-label="t('截图翻译')" @click="capture"><Camera :size="15" /></Button></template>
    </TranslationPanel>
    <p v-else role="status">{{ t('正在读取配置…') }}</p>
  </main>
</template>

<style scoped>
.translation-window { height:100vh; display:flex; flex-direction:column; gap:8px; overflow:auto; padding:0 8px 8px; background:#f5f5f7; color:#27272a; font-size:13px; }
/* 隐藏浮窗边缘的滚动条，保留小窗口和长内容的滚轮、键盘滚动。 */
.translation-window { scrollbar-width:none; }
.translation-window::-webkit-scrollbar { display:none; }
.translation-tools { position:sticky; top:0; z-index:2; background:inherit; display:flex; align-items:center; flex-shrink:0; height:34px; margin:0 -4px -4px; gap:4px; }
.translation-tools :deep(button) { width:28px; min-height:28px; height:28px; padding:5px; color:#71717a; }
.translation-tools :deep(button[aria-pressed=true]) { color:#2563eb; }
.window-drag { flex:1; align-self:stretch; cursor:default; }
.window-error { margin:0; padding:4px; font-size:12px; color:#b42318; overflow-wrap:anywhere; }
:global([data-theme=dark]) .translation-window { background:#18181b; color:#f4f4f5; }
:global([data-theme=dark]) .translation-tools :deep(button) { color:#a1a1aa; }
</style>
