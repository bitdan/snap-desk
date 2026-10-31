<script setup lang="ts">
import { handleWindowEscape as handleEscape } from '@/shared/ui/escape'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Save, X, AlertCircle } from 'lucide-vue-next'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { resolveWindowKind } from '@/shared/window/window-kind'
import { api, onCaptureClosed, onScroll } from '@/services/api'
import type { ScrollView } from '@/features/scroll/types'
import { newerScrollPreview, scrollViewportStyle } from '@/features/scroll/scroll-capture'

const controls = resolveWindowKind(isTauri() ? getCurrentWindow().label : null, location.search) === 'scroll-controls'
const view = ref<ScrollView | null>(null)
const busy = ref(false)
const error = ref('')
const cancelling = ref(false)
// 恢复时仍持续取样，用户滚回重叠区即可接续，无需另点“继续”。
const recovering = computed(() => view.value?.collecting && view.value.status === 'recovering')
const failed = computed(() => view.value?.status === 'failed')
const status = computed(() => busy.value ? (cancelling.value ? '正在取消…' : '正在保存…')
  : !view.value ? '准备中…' : view.value.status === 'failed' ? '输出失败' : view.value.status === 'finishing' ? '正在保存…' : view.value.status === 'sampling' ? '请停一下' : view.value.status === 'no_progress' ? '暂无新增' : view.value.status === 'waiting' ? '请稍候' : view.value.status === 'recovering' ? '请往回滚' : !view.value.collecting ? '采集结束' : '自动拼接')
const hint = computed(() => error.value || view.value?.message || '慢慢滚动，停一下再继续。')
let off: (() => void) | undefined
let offClosed: (() => void) | undefined
let disposed = false
let updates = 0
let closedThroughId = 0
let actionGeneration = 0
function receive(next: ScrollView) {
  if (disposed) return
  const accepted = newerScrollPreview(view.value, next, closedThroughId)
  if (accepted !== next) return
  updates++
  // 新失败版本表示后台已经结束本轮输出，恢复重试/取消入口；重复快照不打断新请求。
  if (next.id !== view.value?.id || (next.status === 'failed' && next.revision > (view.value?.revision ?? -1))) {
    actionGeneration++; busy.value = false; error.value = ''
  }
  view.value = next
}
// 单轴回滚时移动视口框，不重复增长画布。
const marker = computed(() => view.value ? scrollViewportStyle(view.value) : {})
async function finish(cancel: boolean) {
  if (!view.value || (busy.value && (!cancel || cancelling.value))) return
  const id = view.value.id
  const action = ++actionGeneration
  busy.value = true; cancelling.value = cancel; error.value = ''
  // Esc 可中断尚在等待的完成操作；旧请求失败不能覆盖取消状态或下一会话。
  try { await api.controlScroll(id, cancel, !!recovering.value) }
  catch (e) {
    if (actionGeneration === action && view.value?.id === id && cancelling.value === cancel) {
      error.value = String(e); busy.value = false
    }
  }
}
function key(event: KeyboardEvent) {
  if (handleEscape(event, () => { void finish(true) })) return
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.altKey || event.shiftKey
    || !(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'c') return
  // 输入框保留文本复制；预览与控制条共用完成入口，不复制低分辨率预览图。
  if ((event.target as HTMLElement | null)?.closest?.('input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="textbox"]')) return
  if (!view.value) return
  event.preventDefault()
  if (!event.repeat && !busy.value && view.value.status !== 'finishing') void finish(false)
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  try {
    offClosed = await onCaptureClosed(id => {
      closedThroughId = Math.max(closedThroughId, id)
      updates++
      if (view.value && view.value.id <= id) {
        actionGeneration++
        view.value = null; busy.value = false; error.value = ''
      }
    }, controls ? 'scroll-controls' : 'scroll-preview')
    if (disposed) { offClosed(); return }
    off = await onScroll(receive)
    if (disposed) { off(); return }
    const revision = updates
    const current = await api.currentScroll()
    if (!disposed && updates === revision && current) receive(current)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; window.removeEventListener('keydown', key); off?.(); offClosed?.() })
</script>

<template>
  <main class="live-preview" :class="{ controls }">
    <template v-if="!controls">
      <header><strong>{{ t("滚动截图") }}</strong><span v-if="view" :title="t('已拼接 {0} 帧', { 0: view.frames })">{{ view.width }} × {{ view.height }}</span></header>
      <div class="canvas">
        <div v-if="view?.preview" class="image" :style="{ aspectRatio: `${view.width} / ${view.height}`, '--image-ratio': view.width / view.height }">
          <img :src="view.preview" :alt='t("正在拼接的滚动截图")' />
          <div class="current-frame" :style="marker" :title='t("当前采集画面")' :aria-label='t("当前采集画面的位置")' />
        </div>
        <span v-else>{{ t("准备采集…") }}</span>
      </div>
      <p v-if="view" class="coverage">{{ t("已拼接 {0} 帧", { 0: view.frames }) }}</p>
      <p class="preview-hint" :class="{ error: error || failed, recovering }" :role="error || failed ? 'alert' : recovering ? 'status' : undefined" :title="te(hint)">{{ te(hint) }}</p>
    </template>
    <footer v-else>
      <div class="progress" :title="te(hint)">
        <span class="status"><i :class="{ stopped: busy || !view?.collecting }" />{{ t(status) }}</span>
        <small v-if="view" aria-live="polite">{{ te(view.message) || t("{0} 帧 · {1} px", { 0: view.frames, 1: view.height }) }}</small>
      </div>
      <span v-if="error" class="error-indicator" role="alert" :title="te(error)" :aria-label="te(error)"><AlertCircle :size="16" /></span>
      <button class="save" :disabled="busy || !view || view.status === 'finishing'" :title="recovering ? t('保存并复制已拼接内容（Ctrl+C）') : t('完成并复制（Ctrl+C）')" @click="finish(false)"><Save :size="15" />{{ failed ? t("重试") : recovering ? t("保存已确认部分") : t("完成") }}</button>
      <button class="cancel" :disabled="!view || (busy && cancelling)" :title='t("取消（Esc，不保存）")' :aria-label='t("取消滚动截图")' @click="finish(true)"><X :size="17" /><span v-if="recovering || failed">{{ t("取消") }}</span></button>
    </footer>
  </main>
</template>

<style scoped>
.live-preview { height: 100vh; display: flex; flex-direction: column; padding: 6px; gap: 6px; background: #fafafa; border: 1px solid #d4d4d8; color: #27272a; font-size: 12px; overflow: hidden; }
header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 4px; flex: none; padding: 2px; }
header span { color: #71717a; font-size: 10px; }
.canvas { flex: 1; min-height: 0; display: flex; justify-content: center; align-items: center; overflow: hidden; background: #e4e4e7; border-radius: 3px; container-type: size; }
.image { background: repeating-conic-gradient(#d4d4d8 0% 25%, #fafafa 0% 50%) 0 / 12px 12px; position: relative; width: min(100cqw, calc(100cqh * var(--image-ratio))); flex: none; }
img { display: block; width: 100%; height: 100%; }
.current-frame { position: absolute; box-sizing: border-box; border: 2px solid #24834b; background: #84cc1610; pointer-events: none; }
.coverage { margin: 0; flex: none; font-size: 12px; color: #57606a; }
.preview-hint { flex: none; margin: 0; color: #71717a; font-size: 12px; overflow-wrap: anywhere; }
.preview-hint.recovering { white-space: normal; overflow-wrap: anywhere; font-size: 12px; color: #92400e; }
.preview-hint.recovering.error { color: #b91c1c; }
.controls { justify-content: center; padding: 5px 8px; }
footer { display: flex; align-items: center; gap: 7px; min-width: 0; }
.progress { flex: 1; min-width: 0; display: grid; gap: 1px; }
.status { display: flex; align-items: center; gap: 5px; white-space: nowrap; }
.status i { width: 6px; height: 6px; border-radius: 50%; background: #7c3aed; flex: none; }.status i.stopped { background: #a1a1aa; }
small { color: #71717a; font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
button { flex: none; display: flex; align-items: center; justify-content: center; gap: 5px; height: 30px; border-radius: 5px; padding: 0 9px; }
.save { background: #7c3aed; color: white; }.cancel { padding: 0 5px; }.cancel:hover:not(:disabled) { background: #fee2e2; color: #b91c1c; }
button:disabled { opacity: .5; }.error, .error-indicator { color: #b91c1c; }.error-indicator { display: flex; }
button:focus-visible { outline: 2px solid #a78bfa; outline-offset: 1px; }
</style>
