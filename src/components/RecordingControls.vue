<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Pause, Play, Square, X } from 'lucide-vue-next'
import { api } from '@/lib/api'
import RecordingEditor from './RecordingEditor.vue'
import { newerRecording, recordingActive, recordingStatus, recordingTime, type RecordingAction, type RecordingView, type RecordingResult } from '@/lib/recording'

const view = ref<RecordingView | null>(null)
const busy = ref(false)
const error = ref('')
const result = ref<RecordingResult | null>(null)
const loadingPreview = ref(false)
let previewSession: number | null = null
const dismissedSession = ref<number | null>(null)
const replay = computed(() => view.value?.phase === 'completed' && view.value.id !== dismissedSession.value)
/** 成功结果只按原生 ID 打开；旧会话的异步查询不能覆盖下一次录制。 */
async function loadPreview() {
  if (!view.value || loadingPreview.value) return
  const sessionId = view.value.id
  previewSession = sessionId; loadingPreview.value = true; error.value = ''
  try {
    const next = await api.latestRecordingResult()
    if (disposed || view.value?.id !== sessionId) return
    if (!next || next.path !== view.value.path) throw new Error('录屏结果已更新，请重试。')
    result.value = next
  } catch (e) { if (!disposed && view.value?.id === sessionId) error.value = String(e) }
  finally { loadingPreview.value = false }
}
async function closePreview() {
  if (!view.value || busy.value) return
  const id = view.value.id
  busy.value = true; error.value = ''
  try {
    // 已完成会话的 cancel 只收起回放；原生保留已经保存的成品。
    await api.controlRecording(id, 'cancel')
    dismissedSession.value = id; result.value = null
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
let disposed = false
let timer: ReturnType<typeof setTimeout> | undefined
// 控制条窗口复用且保持挂载；新会话清除上次错误，顺序号拦截迟到查询。
async function refresh() {
  const next = await api.currentRecording()
  if (disposed) return
  if (next && next.id !== view.value?.id) { error.value = ''; result.value = null; previewSession = null }

  view.value = newerRecording(view.value, next)
  if (replay.value && previewSession !== view.value?.id) await loadPreview()
}
async function poll() {
  try { await refresh() } catch (e) { if (!disposed) error.value = String(e) }
  if (!disposed) timer = setTimeout(poll, 250)
}
async function control(action: RecordingAction) {
  if (!view.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action); await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
onMounted(poll)
onUnmounted(() => { disposed = true; clearTimeout(timer) })
</script>

<template>
  <main v-if="replay" class="recording-replay" aria-label="录屏回放与编辑">
    <RecordingEditor v-if="result" :key="result.id" :result-id="result.id" :disabled="busy" @close="closePreview" />
    <template v-else>
      <header><strong>录屏回放与编辑</strong><button :disabled="busy" @click="closePreview">关闭</button></header>
      <p v-if="loadingPreview" role="status">正在加载录屏…</p>
      <button v-else @click="loadPreview">重新读取录屏</button>
    </template>
    <p v-if="error" class="preview-error" role="alert">{{ error }}</p>
  </main>
  <main v-else class="recording-controls" aria-label="录屏控制条">
    <span class="dot" :class="{ paused: view?.phase !== 'recording' }" />
    <span class="time" role="timer">{{ recordingTime(view?.seconds ?? 0) }}</span>
    <span class="status" role="status">{{ view ? recordingStatus(view) : '连接中…' }}</span>
    <span v-if="view?.phase !== 'countdown'" class="fps" :title="`近两秒 ${view?.stats.recentFps.toFixed(1) ?? '0.0'} FPS；编码提交 ${view?.frames ?? 0} 帧，队列满 ${view?.stats.queueDroppedFrames ?? 0} 帧，旧代次 ${view?.stats.staleQueuedFrames ?? 0} 帧；静态画面低 FPS 不等同丢帧`">{{ view?.targetFps ?? 30 }} FPS</span>
    <button v-if="view?.phase === 'ready'" class="start" :disabled="busy" title="开始录制" aria-label="开始录制" @click="control('start')"><Play :size="15" />开始</button>
    <button v-else-if="view?.phase === 'countdown'" disabled><Play :size="15" />等待开始</button>
    <button v-else-if="view?.phase === 'paused'" :disabled="busy" title="继续录制" aria-label="继续录制" @click="control('resume')"><Play :size="17" /></button>
    <button v-else :disabled="busy || view?.phase !== 'recording'" title="暂停录制" aria-label="暂停录制" @click="control('pause')"><Pause :size="17" /></button>
    <button :disabled="busy || !view || !['recording', 'paused'].includes(view.phase)" title="结束录制并编辑" aria-label="结束录制并编辑" @click="control('stop')"><Square :size="17" /></button>
    <button :disabled="busy || !recordingActive(view)" title="取消并丢弃本次录屏" aria-label="取消并丢弃本次录屏" @click="control('cancel')"><X :size="17" /></button>
    <span v-if="error" class="error" role="alert" :title="error">{{ error }}</span>
  </main>
</template>

<style scoped>
.recording-replay { height: 100vh; padding: 8px 16px; overflow: auto; background: #fff; color: #24292f; font-size: 13px; border: 1px solid #e1e4e8; border-radius: 7px; }
.recording-replay :deep(.recording-editor) { border: 0; padding: 0; }
.recording-replay header { display: flex; justify-content: space-between; align-items: center; margin: 8px 0; }
.preview-error { color: #b91c1c; overflow-wrap: anywhere; }
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
