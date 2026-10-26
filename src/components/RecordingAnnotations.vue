<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Check, Pause, Play, Square } from 'lucide-vue-next'
import { api } from '@/lib/api'
import { t, te } from '@/lib/i18n'
import { recordingTime, type RecordingView, type RecordingAction } from '@/lib/recording'
import { prepareRecordingRaster, type RecordingAnnotationView } from '@/lib/recording-annotations'
import { useAnnotations } from '@/lib/use-annotations'
import { useFloatingToolbar } from '@/lib/toolbar-layout'
import AnnotationSurface from './AnnotationSurface.vue'
import AnnotationToolbar from './AnnotationToolbar.vue'

const view = ref<RecordingAnnotationView | null>(null)
const recording = ref<RecordingView | null>(null)
const busy = ref(false), error = ref('')
const toolbar = ref<HTMLElement | null>(null)
const { style: toolbarStyle, start: floatToolbar, reset: resetToolbar } = useFloatingToolbar(toolbar)
const bounds = computed(() => ({ x: 0, y: 0, width: view.value?.width ?? 1, height: view.value?.height ?? 1 }))
const editor = useAnnotations(() => bounds.value, () => '', () => busy.value || !view.value?.editing, async shapes => {
  const id = view.value?.id
  if (id === undefined) throw new Error('录制已结束。')
  const next = await api.updateRecordingAnnotations(id, shapes)
  if (!disposed && view.value?.id === id) view.value = { ...view.value, ...next }
}, prepareRecordingRaster)
const stage = computed(() => {
  const v = view.value
  return v ? { left: `${100 * v.x / v.viewportWidth}%`, top: `${100 * v.y / v.viewportHeight}%`,
    width: `${100 * v.width / v.viewportWidth}%`, height: `${100 * v.height / v.viewportHeight}%` } : {}
})
const locked = computed(() => busy.value || editor.pending || editor.interacting)
let disposed = false, closedThrough = -1
let operation = 0
let timer: ReturnType<typeof setTimeout> | undefined
let off: (() => void) | undefined
/** 只在首次加载/外部版本变化时重置历史，轮询不能打断正在画的一笔或撤销栈。 */
async function poll() {
  const known = view.value
  const epoch = operation
  try {
    const [next, status] = await Promise.all([api.recordingAnnotations(known?.revision), api.currentRecording()])
    if (disposed) return
    recording.value = status
    if (!next || next.id <= closedThrough) { view.value = null; editor.reset() }
    else if (epoch !== operation) { /* 操作开始前的查询不覆盖操作结果。 */ }
    else if (!view.value || next.id !== view.value.id) {
      view.value = next; editor.reset(next.shapes ?? []); editor.setTool('pen'); error.value = ''
    } else {
      // 提交期间的查询可能返回旧版本，不可覆盖刚保存的版本或模式。
      if (next.revision >= view.value.revision && !busy.value && !editor.pending) {
        if (next.revision !== view.value.revision) editor.reset(next.shapes ?? [])
        view.value = next
        if (!next.editing) { editor.abort(); editor.selected = -1 }
      }
    }
  } catch (e) { if (!disposed) error.value = String(e) }
  if (!disposed) timer = setTimeout(poll, 250)
}
async function finish() {
  if (!view.value || locked.value) return
  operation++
  busy.value = true; error.value = ''
  try {
    await api.setRecordingAnnotationMode(view.value.id, false)
    if (view.value) view.value.editing = false
    editor.selected = -1
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function control(action: RecordingAction) {
  if (!view.value || locked.value) return
  operation++
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action) }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(event: KeyboardEvent) {
  if (editor.key(event)) return
  if (event.key === 'Escape' && !event.isComposing) { event.preventDefault(); void finish() }
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  try {
    const release = await listen<number>('recording-annotations-closed', event => {
      closedThrough = Math.max(closedThrough, event.payload)
      if (view.value && view.value.id <= closedThrough) { view.value = null; editor.reset() }
    })
    if (disposed) { release(); return }
    off = release
  } catch (e) { error.value = String(e) }
  if (!disposed) void poll()
})
onUnmounted(() => { disposed = true; clearTimeout(timer); off?.(); window.removeEventListener('keydown', key) })
</script>

<template>
  <main v-if="view" class="live-annotations" :aria-label="t('录制中标注')">
    <div class="live-stage" :class="{ editing: view.editing }" :style="stage">
      <AnnotationSurface :editor="editor" :width="view.width" :height="view.height" :bounds="bounds" :enabled="view.editing && !busy" />
    </div>
    <div v-if="view.editing" ref="toolbar" class="live-toolbar" :style="toolbarStyle">
      <AnnotationToolbar :editor="editor" :disabled="busy" :active="true" collapsible @tool="editor.setTool" @float="floatToolbar" @reset-position="resetToolbar">
        <template #leading><span class="recording-time">● {{ recordingTime(recording?.seconds ?? 0) }}</span></template>
        <template #actions>
          <button :disabled="locked" :title="t(recording?.phase === 'paused' ? '继续录制' : '暂停录制')" @click="control(recording?.phase === 'paused' ? 'resume' : 'pause')"><Play v-if="recording?.phase === 'paused'" :size="16" /><Pause v-else :size="16" /></button>
          <button :disabled="locked" :title="t('结束录制并编辑')" @click="control('stop')"><Square :size="16" /></button>
          <button :disabled="locked" @click="finish"><Check :size="16" />{{ t('继续操作桌面') }}</button>
        </template>
      </AnnotationToolbar>
      <p v-if="error || editor.error" role="alert">{{ te(error || editor.error) }}</p>
      <p v-else-if="['mosaic', 'blur', 'magnify'].includes(editor.tool)" role="status">{{ t('滤镜按实时视频画面处理，编辑时显示范围。') }}</p>
    </div>
  </main>
</template>

<style scoped>
.live-annotations { position: fixed; inset: 0; background: transparent; font-size: 13px; }
.live-stage { position: absolute; }
.live-stage.editing { outline: 2px solid #3b82f6; }
.live-toolbar { position: fixed; bottom: 16px; left: 16px; max-width: calc(100vw - 32px); padding: 6px; background: #fff; color: #24292f; border: 1px solid #e1e4e8; border-radius: 6px; box-shadow: 0 3px 12px #0002; }
.live-toolbar :deep(.annotation-tools) { flex-wrap: wrap; }
.recording-time { color: #dc2626; font-variant-numeric: tabular-nums; white-space: nowrap; }
button { display: inline-flex; align-items: center; justify-content: center; gap: 4px; min-height: 30px; padding: 0 8px; border: 1px solid #e1e4e8; border-radius: 4px; }
button:disabled { opacity: .45; }
button:hover:enabled { background: #eff6ff; }
button:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
p { margin: 4px; font-size: 12px; }
[role=alert] { color: #b91c1c; }
:global([data-theme=dark]) .live-toolbar { background: #20252c; color: #e6edf3; border-color: #444c56; }
:global([data-theme=dark]) button { background: #20252c; border-color: #444c56; }
</style>
