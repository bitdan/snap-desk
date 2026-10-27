<script setup lang="ts">
import { useRecordingPolling } from '@/features/recording/use-recording-polling'
import { handleEscape } from '@/shared/ui/escape'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Pause, Play, Square, X } from 'lucide-vue-next'
import { api, onRecordingPreviewClosed } from '@/services/api'
import RecordingEditor from '@/features/recording/components/RecordingEditor.vue'
import { newerRecording, recordingActive, recordingStatus, recordingTime, recordingSpaceAction, type RecordingAction, type RecordingView, type RecordingResult } from '@/features/recording/recording'

const view = ref<RecordingView | null>(null)
const busy = ref(false)
const error = ref('')
const result = ref<RecordingResult | null>(null)
const loadingPreview = ref(false)
let previewSession: number | null = null
let localCloseSession: number | null = null
const dismissedSession = ref<number | null>(null)
const replay = computed(() => view.value?.phase === 'completed' && view.value.id !== dismissedSession.value)
/** 成功结果只按原生 ID 打开；旧会话的异步查询不能覆盖下一次录制。 */
async function loadPreview() {
  if (!view.value || loadingPreview.value) return
  const sessionId = view.value.id
  previewSession = sessionId; loadingPreview.value = true; error.value = ''
  try {
    const next = await api.latestRecordingResult()
    if (disposed || view.value?.id !== sessionId || dismissedSession.value === sessionId) return
    if (!next || next.path !== view.value.path) throw new Error('录屏结果已更新，请重试。')
    result.value = next
  } catch (e) { if (!disposed && view.value?.id === sessionId) error.value = String(e) }
  finally { loadingPreview.value = false }
}
async function closePreview() {
  if (!view.value || busy.value) return
  const id = view.value.id
  busy.value = true; error.value = ''
  localCloseSession = id
  try {
    // 已完成会话的 cancel 只收起回放；原生保留已经保存的成品。
    await api.controlRecording(id, 'cancel')
    dismissedSession.value = id; result.value = null
  } catch (e) { localCloseSession = null; error.value = String(e) }
  finally { busy.value = false }
}
/** 先卸载播放器并收起原选区回放，再进入现有框选流程；已保存的原片始终保留。 */
async function rerecord() {
  const previous = result.value, sessionId = view.value?.id
  if (busy.value) return
  busy.value = true; error.value = ''
  dismissedSession.value = sessionId ?? null; result.value = null
  try {
    // 单条 IPC 在原生侧完成隐藏旧回放、开始框选和成功后释放窗口。
    await api.selectRecordingArea()
  } catch (e) {
    if (!disposed && view.value?.id === sessionId) { dismissedSession.value = null; result.value = previous; error.value = String(e) }
  } finally { busy.value = false }
}
let disposed = false
// 按需挂载先查询原生快照，不依赖窗口创建前可能错过的事件。
async function refresh() {
  const next = await api.currentRecording()
  if (disposed) return
  if (next && next.id !== view.value?.id) { error.value = ''; result.value = null; previewSession = null }

  view.value = newerRecording(view.value, next)
  if (replay.value && previewSession !== view.value?.id) await loadPreview()
}
useRecordingPolling(refresh, () => view.value?.phase, e => { if (!disposed) error.value = String(e) })
async function control(action: RecordingAction) {
  if (!view.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action); await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
let offPreview: (() => void) | undefined
/** 录屏不能按普通窗口直接隐藏：Esc 结束并保留成品，回放时仅收起回放。 */
function key(event: KeyboardEvent) {
  if (handleEscape(event, () => {
    if (busy.value || !view.value) return
    if (replay.value) void closePreview()
    else if (['recording', 'paused'].includes(view.value.phase)) void control('stop')
    else if (['ready', 'countdown'].includes(view.value.phase)) void control('cancel')
  })) return
  const action = recordingSpaceAction(event, view.value?.phase)
  if (action) { event.preventDefault(); void control(action) }
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  try {
    const off = await onRecordingPreviewClosed(id => {
      // 本地按钮已负责卸载；忽略它对应的异步通知，避免重新框选失败恢复后被迟到事件关闭。
      if (localCloseSession === id) { localCloseSession = null; return }
      if (view.value && view.value.id !== id) return
      dismissedSession.value = id; result.value = null
    })
    if (disposed) { off(); return }
    offPreview = off
  } catch (e) { if (!disposed) error.value = String(e) }
})
onUnmounted(() => { disposed = true; window.removeEventListener('keydown', key); offPreview?.() })
</script>

<template>
  <main v-if="replay" class="recording-replay" :aria-label='t("录屏回放与编辑")'>
    <RecordingEditor v-if="result" :key="result.id" :result-id="result.id" :disabled="busy" floating :layout="result.replayLayout" :frame-rate="view?.targetFps ?? 30" @close="closePreview" @rerecord="rerecord" />
    <template v-else>
      <header><strong>{{ t("录屏回放与编辑") }}</strong><button :disabled="busy" @click="closePreview">{{ t("关闭") }}</button></header>
      <p v-if="loadingPreview" role="status">{{ t("正在加载录屏…") }}</p>
      <button v-else @click="loadPreview">{{ t("重新读取录屏") }}</button>
    </template>
    <p v-if="error" class="preview-error" role="alert">{{ te(error) }}</p>
  </main>
  <main v-else class="recording-controls" :aria-label='t("录屏控制条")'>
    <span class="dot" :class="{ paused: view?.phase !== 'recording' }" />
    <span class="time" role="timer">{{ recordingTime(view?.seconds ?? 0) }}</span>
    <span class="status" role="status">{{ view ? t(recordingStatus(view)) : t("连接中…") }}</span>
    <span v-if="view?.phase !== 'countdown'" class="fps">{{ view?.targetFps ?? 30 }} FPS</span>
    <button v-if="view?.phase === 'ready'" class="start" :disabled="busy" :title='t("开始录制")' :aria-label='t("开始录制")' @click="control('start')"><Play :size="15" />{{ t("开始") }}</button>
    <button v-else-if="view?.phase === 'countdown'" disabled><Play :size="15" />{{ t("等待开始") }}</button>
    <button v-else-if="view?.phase === 'paused'" :disabled="busy" :title='t("继续录制")' :aria-label='t("继续录制")' @click="control('resume')"><Play :size="17" /></button>
    <button v-else :disabled="busy || view?.phase !== 'recording'" :title='t("暂停录制")' :aria-label='t("暂停录制")' @click="control('pause')"><Pause :size="17" /></button>
    <button :disabled="busy || !view || !['recording', 'paused'].includes(view.phase)" :title='t("结束录制并编辑")' :aria-label='t("结束录制并编辑")' @click="control('stop')"><Square :size="17" /></button>
    <button :disabled="busy || !recordingActive(view)" :title='t("取消并丢弃本次录屏")' :aria-label='t("取消并丢弃本次录屏")' @click="control('cancel')"><X :size="17" /></button>
    <span v-if="error" class="error" role="alert" :title="te(error)">{{ te(error) }}</span>
  </main>
</template>

<style scoped>
.recording-replay { height: 100vh; overflow: hidden; color: #24292f; font-size: 13px; background: transparent; }
.recording-replay > header, .recording-replay > p, .recording-replay > button { background: #fff; }
.recording-replay header { display: flex; justify-content: space-between; align-items: center; margin: 8px 0; }
.preview-error { position:absolute; left:8px; right:8px; bottom:4px; z-index:3; margin:0; background:#fff; color:#b91c1c; overflow-wrap:anywhere; }
.recording-controls { height: 100vh; display: flex; align-items: center; justify-content: center; gap: 8px; padding: 5px 8px; background: #fafafa; border: 1px solid #ddd; border-radius: 7px; color: #27272a; font-size: 12px; overflow: hidden; }
.dot { flex: none; width: 8px; height: 8px; border-radius: 50%; background: #ef4444; }
.dot.paused { background: #a1a1aa; }
.time { font-variant-numeric: tabular-nums; white-space: nowrap; }
.status { min-width: 42px; color: #71717a; white-space: nowrap; }
button { min-width: 30px; display: flex; align-items: center; justify-content: center; gap: 3px; flex: none; padding: 0 7px; height: 30px; border: 1px solid #ddd; border-radius: 5px; white-space: nowrap; }
.start { background: #7c3aed; color: white; border-color: #7c3aed; }
.fps { color: #71717a; font-size: 11px; white-space: nowrap; }
button:hover { background: #ede9fe; color: #7c3aed; }
button:disabled { opacity: .4; }
.error { position: absolute; bottom: 0; left: 4px; right: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #b91b1b; font-size: 10px; }
</style>
