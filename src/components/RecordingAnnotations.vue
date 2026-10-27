<script setup lang="ts">
import { handleEscape } from '@/lib/escape'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Pause, Play, Square } from 'lucide-vue-next'
import { api } from '@/lib/api'
import { t, te } from '@/lib/i18n'
import { recordingTime, recordingSpaceAction, type RecordingView, type RecordingAction } from '@/lib/recording'
import { prepareRecordingRaster, type RecordingAnnotationView } from '@/lib/recording-annotations'
import { useAnnotations, type AnnotationEditor, type AnnotationTool } from '@/lib/use-annotations'
import { annotationToolbarPosition, useFloatingToolbar } from '@/lib/toolbar-layout'
import AnnotationWorkspace from './AnnotationWorkspace.vue'

const props = defineProps<{ sessionId?: number; initialEditor?: AnnotationEditor }>()
const emit = defineEmits<{ closed: [] }>()
const view = ref<RecordingAnnotationView | null>(null)
const recording = ref<RecordingView | null>(null)
const busy = ref(false), error = ref('')
const toolbar = ref<HTMLElement | null>(null)
const { style: toolbarStyle, start: floatToolbar, reset: resetToolbar } = useFloatingToolbar(toolbar)
const viewport = ref({ x: 0, y: 0, width: innerWidth, height: innerHeight })
const toolbarSize = ref({ width: 800, height: 80 })
watch(toolbar, (element, _, cleanup) => {
  if (!element) return
  const observer = new ResizeObserver(() => { toolbarSize.value = { width: element.offsetWidth, height: element.offsetHeight } })
  observer.observe(element); cleanup(() => observer.disconnect())
})
const defaultToolbarStyle = computed(() => {
  const v = view.value, display = viewport.value
  if (!v) return {}
  const sx = display.width / v.viewportWidth, sy = display.height / v.viewportHeight
  return annotationToolbarPosition({ x: v.x * sx, y: v.y * sy, width: v.width * sx, height: v.height * sy }, display, toolbarSize.value)
})
function resized() { viewport.value = { x: 0, y: 0, width: innerWidth, height: innerHeight } }
const bounds = computed(() => ({ x: 0, y: 0, width: view.value?.width ?? 1, height: view.value?.height ?? 1 }))
const editor = useAnnotations(() => bounds.value, () => '', () => busy.value || !view.value, async shapes => {
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
    if (!next || next.id <= closedThrough || (props.sessionId !== undefined && next.id !== props.sessionId)) { view.value = null; editor.reset() }
    else if (epoch !== operation) { /* 操作开始前的查询不覆盖操作结果。 */ }
    else if (!view.value || next.id !== view.value.id) {
      view.value = next; editor.reset(next.shapes ?? []); editor.setTool(props.initialEditor?.tool ?? 'edit'); error.value = ''
      if (props.initialEditor) for (const key of ['color', 'width', 'opacity', 'magnification', 'fontSize', 'fontFamily'] as const) {
        Object.assign(editor, { [key]: props.initialEditor[key] })
      }
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
let activation: Promise<void> | null = null
/** 在原工具栏直接选工具即可恢复绘制；桌面操作期间不另开一块控制界面。 */
async function activate() {
  if (!view.value || view.value.editing || busy.value) return
  if (!activation) {
    operation++
    const id = view.value.id
    activation = api.setRecordingAnnotationMode(id, true).then(() => {
      if (view.value?.id === id) view.value.editing = true
    }).catch(e => { error.value = String(e) }).finally(() => { activation = null })
  }
  await activation
}
async function chooseTool(tool: AnnotationTool) { await activate(); editor.setTool(tool) }
function moveToolbar(event: PointerEvent) { void activate(); floatToolbar(event) }
async function control(action: RecordingAction) {
  if (!view.value || locked.value) return
  operation++
  busy.value = true; error.value = ''
  try {
    const id = view.value.id
    await api.controlRecording(id, action)
    if (action === 'stop' || action === 'cancel') {
      // 原生停止会同时隐藏交互窗口；本地清除整套工具栏，迟到轮询不能重新挂回。
      closedThrough = Math.max(closedThrough, id); view.value = null; editor.reset(); emit('closed')
    } else {
      const next = await api.currentRecording()
      if (!disposed && view.value?.id === next?.id) recording.value = next
    }
  }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(event: KeyboardEvent) {
  if (handleEscape(event, () => {
    if (busy.value || editor.pending) return
    editor.abort(); void control(recording.value?.phase === 'ready' || recording.value?.phase === 'countdown' ? 'cancel' : 'stop')
  })) return
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return
  const action = recordingSpaceAction(event, recording.value?.phase)
  if (action) { event.preventDefault(); void control(action); return }
  if (editor.key(event)) return
}
onMounted(async () => {
  window.addEventListener('keydown', key)
  window.addEventListener('resize', resized)
  try {
    const release = await listen<number>('recording-annotations-closed', event => {
      closedThrough = Math.max(closedThrough, event.payload)
      if (view.value && view.value.id <= closedThrough) { view.value = null; editor.reset() }
      if (props.sessionId === event.payload) emit('closed')
    })
    if (disposed) { release(); return }
    off = release
  } catch (e) { error.value = String(e) }
  if (!disposed) void poll()
})
onUnmounted(() => { disposed = true; clearTimeout(timer); off?.(); window.removeEventListener('keydown', key); window.removeEventListener('resize', resized) })
</script>

<template>
  <main v-if="view" class="live-annotations" :aria-label="t('录制中标注')">
    <AnnotationWorkspace :key="view.id" default-expanded :editor="editor" :width="view.width" :height="view.height" :bounds="bounds" :surface-visible="view.editing" :toolbar-visible="true" :enabled="view.editing && !busy" :active="true" :disabled="busy" :surface-style="stage" :outlined="view.editing" :toolbar-style="[defaultToolbarStyle, toolbarStyle]" @toolbar-element="toolbar = $event" @activate="activate" @tool="chooseTool" @float="moveToolbar" @reset-position="resetToolbar">
        <template #leading><span class="recording-time">● {{ recordingTime(recording?.seconds ?? 0) }}</span></template>
        <template #actions>
          <button v-if="recording?.phase === 'ready'" :disabled="locked" :title="t('开始录制')" @click="control('start')"><Play :size="16" />{{ t('开始录制') }}</button>
          <button v-else :disabled="locked || !recording || !['recording', 'paused'].includes(recording.phase)" :title="t(recording?.phase === 'paused' ? '继续录制' : '暂停录制')" @click="control(recording?.phase === 'paused' ? 'resume' : 'pause')"><Play v-if="recording?.phase === 'paused'" :size="16" /><Pause v-else :size="16" /></button>
          <button :disabled="locked || !recording || !['recording', 'paused'].includes(recording.phase)" :title="t('结束录制并编辑')" @click="control('stop')"><Square :size="16" /></button>
        </template>
      <template #footer>
      <p v-if="error || editor.error" role="alert">{{ te(error || editor.error) }}</p>
      <p v-else-if="['mosaic', 'blur', 'magnify'].includes(editor.tool)" role="status">{{ t('滤镜按实时视频画面处理，编辑时显示范围。') }}</p>
      </template>
    </AnnotationWorkspace>
  </main>
</template>

<style scoped>
.live-annotations { position: fixed; inset: 0; background: transparent; font-size: 13px; }
.recording-time { color: #dc2626; font-variant-numeric: tabular-nums; white-space: nowrap; }
p { margin: 4px; font-size: 12px; }
[role=alert] { color: #b91c1c; }
</style>
