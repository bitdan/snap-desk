<script setup lang="ts">
import { useRecordingPolling } from '@/features/recording/use-recording-polling'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref, unref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Pause, Play, Square } from 'lucide-vue-next'
import { api } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import { recordingTime, recordingSpaceAction, type RecordingView, type RecordingAction } from '@/features/recording/recording'
import { prepareRecordingRaster, recordingToolbarRegion, type RecordingAnnotationView } from '@/features/recording/recording-annotations'
import { useAnnotations, type AnnotationEditor, type AnnotationTool } from '@/features/annotations/use-annotations'
import { annotationToolbarPosition, useFloatingToolbar } from '@/features/annotations/toolbar-layout'
import AnnotationWorkspace from '@/features/annotations/components/AnnotationWorkspace.vue'
import type { Rect } from '@/shared/geometry/selection'

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
let off: (() => void) | undefined
/** 只在首次加载/外部版本变化时重置历史，轮询不能打断正在画的一笔或撤销栈。 */
async function poll() {
  const known = view.value
  const epoch = operation
  try {
    const [next, status] = await Promise.all([api.recordingAnnotations(known?.revision), api.currentRecording()])
    if (disposed || epoch !== operation) return
    recording.value = status
    if (!next || next.id <= closedThrough || (props.sessionId !== undefined && next.id !== props.sessionId)) { view.value = null; editor.reset() }
    else if (!view.value || next.id !== view.value.id) {
      appliedMode = ''
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
}
useRecordingPolling(poll, () => recording.value?.phase, e => { if (!disposed) error.value = String(e) }, { initialSnapshot: true })
let activation: Promise<void> | null = null
// 原生模式请求按顺序执行，旧的工具栏裁剪不能晚于恢复绘制而重新截断画布。
let modeQueue = Promise.resolve()
let appliedMode = ''
function setMode(id: number, editing: boolean, region?: Rect, deduplicate = false) {
  const signature = JSON.stringify([id, editing, region])
  const request = modeQueue.then(async () => {
    if (disposed || view.value?.id !== id) return
    // set_size 可能再次触发 resize，同一裁剪不重复提交以免形成窗口更新循环。
    if (deduplicate && appliedMode === signature) return
    await api.setRecordingAnnotationMode(id, editing, region)
    // 切换过程中发出的查询也可能仍读取旧模式，完成后让这些快照全部失效。
    operation++
    appliedMode = signature
  })
  modeQueue = request.catch(() => {})
  return request
}
function toolbarRegion() {
  if (!view.value || !toolbar.value) throw new Error('录屏工具栏尚未就绪。')
  return recordingToolbarRegion(toolbar.value.getBoundingClientRect(), viewport.value,
    { width: view.value.viewportWidth, height: view.value.viewportHeight })
}
/** 保留工具栏的原生命中区域，其余桌面交还给正在录制的应用。 */
async function desktop() {
  if (!view.value) return
  const id = view.value.id
  await nextTick()
  await setMode(id, false, toolbarRegion())
  if (view.value?.id === id) { view.value.editing = false; editor.abort(); editor.selected = -1 }
}
/** 由截图宿主在确认范围后调用：先渲染真实工具栏，再以桌面操作模式显示原生窗口。 */
async function prepareDesktop() {
  await poll()
  await nextTick()
  if (!view.value) throw new Error(error.value || '录屏工具栏尚未就绪。')
  await desktop()
}
defineExpose({ prepareDesktop })
async function deactivate() {
  if (locked.value || activation) return
  operation++; busy.value = true; error.value = ''
  try { await desktop() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
// 样式行、错误提示、DPI 和工具栏位置变化后同步裁剪，避免按钮被旧区域截掉。
watch([() => view.value?.editing, toolbar, toolbarSize, viewport, defaultToolbarStyle, () => unref(toolbarStyle)], async () => {
  // 首次接管由宿主等工具栏渲染完成后发起，不能用 initialize 的临时状态抢先操作窗口。
  if (!appliedMode || !view.value || view.value.editing || activation) return
  const id = view.value.id
  try {
    await nextTick()
    if (view.value?.id !== id || view.value.editing || activation) return
    await setMode(id, false, toolbarRegion(), true)
  }
  catch (e) { if (!disposed) error.value = String(e) }
}, { flush: 'post' })
/** 在原工具栏直接选工具即可恢复绘制；桌面操作期间不另开一块控制界面。 */
async function activate() {
  if (!view.value || view.value.editing || busy.value) return
  if (!activation) {
    operation++
    const id = view.value.id
    activation = setMode(id, true).then(() => {
      if (view.value?.id === id) view.value.editing = true
    }).catch(e => { error.value = String(e) }).finally(() => { activation = null })
  }
  await activation
}
/** 重复点击当前工具即可退出绘制，不再增加单独的桌面操作按钮。 */
async function chooseTool(tool: AnnotationTool) {
  // 录屏中的指针直接操作原应用；截图和贴图仍由原宿主将 edit 用于编辑标注。
  if (tool === 'edit') { await deactivate(); if (!view.value?.editing) editor.setTool('edit'); return }
  if (view.value?.editing && editor.tool === tool) { await deactivate(); return }
  await activate()
  if (view.value?.editing) editor.setTool(tool)
}
function moveToolbar(event: PointerEvent) { floatToolbar(event) }
async function control(action: RecordingAction) {
  if (!view.value || locked.value) return
  operation++
  busy.value = true; error.value = ''
  try {
    const id = view.value.id
    // 开始/继续录制默认操作桌面；只有显式选择标注工具才拦截全屏输入。
    if (action === 'start' || action === 'resume') await desktop()
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
})
onUnmounted(() => { disposed = true; off?.(); window.removeEventListener('keydown', key); window.removeEventListener('resize', resized) })
</script>

<template>
  <main v-if="view" class="live-annotations" :aria-label="t('录制中标注')">
    <AnnotationWorkspace :key="view.id" default-expanded :pointer-label="t('操作桌面')" :editor="editor" :width="view.width" :height="view.height" :bounds="bounds" :surface-visible="view.editing" :toolbar-visible="true" :enabled="view.editing && !busy" :active="view.editing" :disabled="busy" :surface-style="stage" :outlined="view.editing" :toolbar-style="[defaultToolbarStyle, toolbarStyle]" @toolbar-element="toolbar = $event" @activate="activate" @tool="chooseTool" @float="moveToolbar" @reset-position="resetToolbar">
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
