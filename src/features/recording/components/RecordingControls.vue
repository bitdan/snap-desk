<script setup lang="ts">
import { useRecordingPolling } from '@/features/recording/use-recording-polling'
import { handleWindowEscape as handleEscape } from '@/shared/ui/escape'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Pause, Play, Square, X, Pencil, Trash2 } from 'lucide-vue-next'
import { api } from '@/services/api'
import type { RecordingAnnotationView, RecordingAnnotationEditorAction } from '../recording-annotations'
import { newerRecording, recordingActive, recordingStatus, recordingTime, recordingSpaceAction, type RecordingAction, type RecordingView } from '@/features/recording/recording'

const view = ref<RecordingView | null>(null)
/** 队列水位只在出现积压/丢帧时通过提示说明，常态不额外占用控制条宽度。 */
const queueHint = computed(() => {
  const stats = view.value?.stats
  if (!stats) return null
  if (stats.queueDroppedFrames > 0) return { key: '编码队列已满，丢弃 {0} 帧', values: { 0: stats.queueDroppedFrames } }
  if (stats.queueBackpressureSkips > 0) return { key: '因编码积压暂停取样 {0} 次', values: { 0: stats.queueBackpressureSkips } }
  if (stats.queuePeak > 0) return { key: '编码队列峰值 {0} 帧', values: { 0: stats.queuePeak } }
  return null
})
const annotation = ref<RecordingAnnotationView | null>(null)
const busy = ref(false)
const error = ref('')
let disposed = false
// 按需挂载先查询原生快照，不依赖窗口创建前可能错过的事件。
async function refresh() {
  const next = await api.currentRecording()
  if (disposed) return
  if (next && next.id !== view.value?.id) error.value = ''

  view.value = newerRecording(view.value, next)
  if (view.value && ['ready','countdown','recording','paused'].includes(view.value.phase)) {
    const status=await api.recordingAnnotations(annotation.value?.revision)
    if (!disposed && status?.id === view.value.id) {
      const sameRevision=annotation.value?.id===status.id && annotation.value.revision===status.revision
      annotation.value={...status,shapes:status.shapes ?? (sameRevision ? annotation.value?.shapes : [])}
    }
  } else annotation.value=null
}
useRecordingPolling(refresh, () => view.value?.phase, e => { if (!disposed) error.value = String(e) }, { initialSnapshot: true })
async function control(action: RecordingAction) {
  if (!view.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action); await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 控制条只切换原生标注模式，不持有或复制标注数据。 */
async function annotate() {
  if (!view.value || busy.value) return
  const id = view.value.id
  busy.value = true; error.value = ''
  try { await api.setRecordingAnnotationMode(id, !annotation.value?.editing); await refresh() }
  catch (e) { if (!disposed && view.value?.id === id) error.value = String(e) }
  finally { busy.value = false }
}
/** 清空只影响之后显示和编码的标注，已录制帧持有原有快照。 */
async function clearAnnotations() {
  if (!view.value || busy.value) return
  const id = view.value.id
  busy.value = true; error.value = ''
  try { await api.recordingAnnotationEditorAction(id,{kind:'clear'}); await refresh() }
  catch (e) { if (!disposed && view.value?.id === id) error.value = String(e) }
  finally { busy.value = false }
}
async function executeEditorAction(action:RecordingAnnotationEditorAction) {
  if(!view.value || busy.value) return
  const id=view.value.id
  busy.value=true;error.value=''
  try {await api.recordingAnnotationEditorAction(id,action);await refresh()}
  catch(e){error.value=String(e)}finally{busy.value=false}
}
/** 录屏不能按普通窗口直接隐藏：Esc 结束录制并直接保存成品。 */
function key(event: KeyboardEvent) {
  if (handleEscape(event, () => {
    if (busy.value || !view.value) return
    if (annotation.value?.editing) void annotate()
    else if (['recording', 'paused'].includes(view.value.phase)) void control('stop')
    else if (['ready', 'countdown'].includes(view.value.phase)) void control('cancel')
  })) return
  if(annotation.value?.editing && !(event.target as HTMLElement)?.closest?.('input,textarea,select,[contenteditable=true]')) {
    const command=event.ctrlKey || event.metaKey
    const key=event.key.toLowerCase()
    if(command && (key==='z' || key==='y')) {
      event.preventDefault()
      if(!event.repeat) void executeEditorAction({kind:key==='y'||event.shiftKey?'redo':'undo'})
      return
    }
    if(key==='delete' || key==='backspace') {
      event.preventDefault(); if(!event.repeat) void executeEditorAction({kind:'remove'}); return
    }
  }
  const action = recordingSpaceAction(event, view.value?.phase)
  if (action) { event.preventDefault(); void control(action) }
}
onMounted(() => window.addEventListener('keydown', key))
onUnmounted(() => { disposed = true; window.removeEventListener('keydown', key) })
</script>

<template>
  <main v-if="view && !['completed', 'cancelled', 'failed'].includes(view.phase)" class="recording-controls" :aria-label='t("录屏控制条")'>
    <div class="recording-transport">
    <span class="dot" :class="{ paused: view?.phase !== 'recording' }" />
    <span class="time" role="timer">{{ recordingTime(view?.seconds ?? 0) }}</span>
    <span class="status" role="status">{{ view ? t(recordingStatus(view)) : t("连接中…") }}</span>
    <span class="divider" />
    <span v-if="view?.phase !== 'countdown'" class="fps" :class="{ dropped: !!view?.droppedFrames }" :title="queueHint ? t(queueHint.key, queueHint.values) : undefined">{{ view?.targetFps ?? 30 }} FPS<template v-if="view?.droppedFrames"> · {{ t("丢帧 {0}", { 0: view.droppedFrames }) }}</template></span>
    <button v-if="view?.phase === 'ready'" class="start" :disabled="busy" :title='t("开始录制")' :aria-label='t("开始录制")' @click="control('start')"><Play :size="15" />{{ t("开始") }}</button>
    <button v-else-if="view?.phase === 'countdown'" disabled><Play :size="15" />{{ t("等待开始") }}</button>
    <button v-else-if="view?.phase === 'paused'" :disabled="busy" :title='t("继续录制")' :aria-label='t("继续录制")' @click="control('resume')"><Play :size="17" /></button>
    <button v-else :disabled="busy || view?.phase !== 'recording'" :title='t("暂停录制")' :aria-label='t("暂停录制")' @click="control('pause')"><Pause :size="17" /></button>
    <button class="stop" :disabled="busy || !view || !['recording', 'paused'].includes(view.phase)" :title='t("结束录制并保存")' :aria-label='t("结束录制并保存")' @click="control('stop')"><Square :size="17" /></button>
    <button :disabled="busy || !recordingActive(view)" :title='t("取消并丢弃本次录屏")' :aria-label='t("取消并丢弃本次录屏")' @click="control('cancel')"><X :size="17" /></button>
    <button :disabled="busy || !view || !['ready', 'countdown', 'recording', 'paused'].includes(view.phase)" :title="t('录制中标注')" :aria-label="t('录制中标注')" @click="annotate"><Pencil :size="17" /></button>
    <button v-if="!annotation?.editing" :disabled="busy || !view || !['ready', 'countdown', 'recording', 'paused'].includes(view.phase)" :title="t('清空标注')" :aria-label="t('清空标注')" @click="clearAnnotations"><Trash2 :size="17" /></button>
    </div>
    <span v-if="error || annotation?.editor?.error" class="error" role="alert" :title="te(error || annotation?.editor?.error || '')">{{ te(error || annotation?.editor?.error || '') }}</span>
  </main>
</template>

<style scoped>
/* 与截图、贴图同款胶囊；透明窗口 480×48，胶囊铺满窗口，不画窗口外阴影。 */
.recording-controls { --tb-surface: #ffffff; --tb-border: #dfe3e8; --tb-text: #24292f; --tb-muted: #6b7280; --tb-hover: #f1f3f5; --tb-divider: #e5e7eb;
  position:relative; height:100vh; overflow:hidden; background:transparent; color:var(--tb-text); font-size:12px; }
:global([data-theme=dark]) .recording-controls { --tb-surface: #20252c; --tb-border: #39424e; --tb-text: #e6edf3; --tb-muted: #9aa7b4; --tb-hover: #2b323c; --tb-divider: #39424e; }
.recording-transport { height:100%; display:flex; align-items:center; justify-content:center; gap:2px; padding:0 6px 0 12px; background:var(--tb-surface); border:1px solid var(--tb-border); border-radius:10px; overflow:hidden; }
.dot { flex:none; width:10px; height:10px; margin-right:6px; border-radius:50%; background:#e5484d; box-shadow:0 0 0 3px #e5484d33; }
.dot.paused { background:#a1a1aa; box-shadow:none; }
.time { margin-right:6px; font-size:14px; font-variant-numeric:tabular-nums; white-space:nowrap; }
.status { min-width:42px; color:var(--tb-muted); white-space:nowrap; }
.divider { flex:none; width:1px; height:18px; margin:0 6px; background:var(--tb-divider); }
.fps { margin-right:auto; color:var(--tb-muted); font-size:11px; white-space:nowrap; }
.fps.dropped { color:#b45309; }
button { min-width:30px; display:flex; align-items:center; justify-content:center; gap:4px; flex:none; padding:0 6px; height:30px; border:0; border-radius:6px; background:transparent; color:inherit; white-space:nowrap; }
button:hover:enabled { background:var(--tb-hover); }
button:disabled { opacity:.4; }
button:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.start { padding:0 12px; background:#2563eb; color:white; }
.start:hover:enabled { background:#1d4ed8; }
.stop { color:#e5484d; }
.error { position:absolute; bottom:1px; left:12px; right:12px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:#b91b1b; font-size:10px; }
</style>
