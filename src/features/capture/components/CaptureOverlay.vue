<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import { handleWindowEscape as handleEscape } from '@/shared/ui/escape'
import { t, te } from '@/shared/i18n/i18n'
import { convertFileSrc } from '@tauri-apps/api/core'
import ScrollCaptureIcon from '@/features/scroll/components/ScrollCaptureIcon.vue'
import QrResultCard from '@/features/recognition/components/QrResultCard.vue'
import OcrWorkspace from '@/features/recognition/components/OcrWorkspace.vue'
import { ocrWindowSize } from '@/features/recognition/ocr-workspace'
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { Copy, Save, X, Monitor, RotateCcw, Scan, Pin, Video, QrCode, ScanText, Languages, AppWindow, GripVertical, Check, MousePointer2, Volume2, VolumeX, Mic, MicOff, ArrowLeft } from 'lucide-vue-next'
import { api, onFrame, onCaptureClosed, onCapturePin, onScroll } from '@/services/api'
import type { CaptureFrame, ColorFormat } from '@/features/capture/types'
import type { ScrollView } from '@/features/scroll/types'
import { type Handle, type Point, type Rect } from '@/shared/geometry/selection'
import { useAnnotations, type AnnotationTool } from '@/features/annotations/use-annotations'
import type { Annotation } from '@/features/annotations/annotations'
import AnnotationWorkspace from '@/features/annotations/components/AnnotationWorkspace.vue'
import RecordingAnnotations from '@/features/recording/components/RecordingAnnotations.vue'
import { annotationToolbarPosition, useFloatingToolbar } from '@/features/annotations/toolbar-layout'
import CaptureMagnifier from '@/features/capture/components/CaptureMagnifier.vue'
import { presentCaptureImage } from '@/features/capture/capture-paint'
import { magnifierSample } from '@/features/capture/magnifier'
import { newerScroll } from '@/features/scroll/scroll-capture'
import RecordingSetupMenu from '@/features/recording/components/RecordingSetupMenu.vue'
import RecordingSourceSelect from '@/features/recording/components/RecordingSourceSelect.vue'
import { fromPixels } from '@/features/capture/capture-targets'
import { useCaptureSelection } from '@/features/capture/use-capture-selection'
import { useCaptureActions } from '@/features/capture/use-capture-actions'
import { useCaptureRecognition } from '@/features/capture/use-capture-recognition'
import { useCaptureRecording } from '@/features/capture/use-capture-recording'

// 截图帧是不可变快照，大量窗口目标不需要逐项响应式代理。
const frame = shallowRef<CaptureFrame | null>(null)
// 选区确认后仅替换背景和运行状态，继续使用当前 capture WebView，不弹出中间控制窗口。
const liveRecordingId = ref<number | null>(null)
const liveAnnotations = ref<InstanceType<typeof RecordingAnnotations> | null>(null)
watch(liveRecordingId, id => document.documentElement.classList.toggle('recording-live', id !== null), { flush: 'sync' })
// 只传会话编号与版本；大图通过受限协议读取，URL 不包含本地文件路径。
const imageUrl = computed(() => frame.value ? `${convertFileSrc(String(frame.value.id), 'capture-image')}/${frame.value.previewRevision}.png` : '')
const captureRoot = ref<HTMLElement | null>(null)
const busy = ref(false)
const error = ref('')
const scroll = ref<ScrollView | null>(null)
const viewport = ref({ width: innerWidth, height: innerHeight })
const annotating = ref(false)
const colorCopied = ref(false)
// 录屏模式由宿主提前建立：选区会话在 watch 首次求值时就需要读取它。
const recordingMode = ref(false)
// 选区会话先建立；标注编辑器依赖 pixels，通过延迟回调访问，避免初始化循环。
const {
  selection, selectedTarget, windowsOnly, hoverLevel, pointer, recentSelection, recentRect, gesture,
  hoverCandidates, hoverTarget, hoverStyle, pixels, valid, boxStyle, activeDisplay, pointerDisplay,
  down, drag, up, abortGesture, reselect, restoreSelection, selectAll, nudge, remember,
} = useCaptureSelection({
  root: captureRoot, frame, viewport, busy, error, colorCopied, annotating,
  getEditor: () => editor,
  marked: () => marked.value,
  recordingMode,
  onRecordingRegion: () => { void selectedRecordingRegion() },
  onAutoTranslate: () => { if (frame.value?.translation && valid.value) void recognizeText(true) },
})
const screenImage = shallowRef<HTMLImageElement | null>(null)
const imageReady = ref(false)
let imageReceivedAt = performance.now()
watch(imageUrl, () => { imageReceivedAt = performance.now(); imageReady.value = false }, { flush: 'sync' })
const relativeCoordinates = ref(false)
const colorFormat = ref<ColorFormat>('rgb')
const editor = useAnnotations(() => pixels.value ?? { x: 0, y: 0, width: 0, height: 0 }, () => imageUrl.value, () => busy.value)
const marked = computed(() => editor.history.present.length > 0)
/** 导出、回放与识别共用互斥条件；标注交互或选区拖动期间不接受新的整图操作。 */
const blocked = computed(() => busy.value || editor.pending || editor.interacting || !!gesture.value)
const { finish, cancel, startScroll } = useCaptureActions({
  frame, pixels, valid, busy, error, scroll,
  blocked: () => blocked.value,
  marked: () => marked.value,
  editorPending: () => editor.pending,
  recognitionOpen: () => !!qrResult.value || ocrResult.value !== null,
  annotations: () => editor.history.present,
  releaseFrame,
  remember,
})
const showMagnifier = computed(() => frame.value && imageReady.value && screenImage.value && pointer.value && !busy.value && !recordingMode.value && (!annotating.value && !marked.value))
/** 原生抓取失败保留原图与选区；成功后等待图片加载再显示蒙层。 */
async function captureNative() {
  if (!frame.value || selectedTarget.value === null || busy.value || marked.value) return
  busy.value = true; error.value = ''
  const id = frame.value.id
  try { const next = await api.nativeWindow(id, selectedTarget.value); if (!disposed && frame.value?.id === id) frame.value = next }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
const handles: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']
// 尺寸标签默认贴在选区上方外侧；顶部放不下时，大选区退回内部，小选区改放左右外侧，避免盖住内容。
const SIZE_LABEL = { width: 190, height: 22, gap: 6 }
const sizeLabelStyle = computed(() => {
  const s = selection.value
  if (!s) return {}
  const d = activeDisplay.value
  const overflowRight = s.x + SIZE_LABEL.width > d.x + d.width
  const alignX = overflowRight ? { left: 'auto', right: '0' } : { left: '0', right: 'auto' }
  if (s.y - d.y >= SIZE_LABEL.height + SIZE_LABEL.gap) return { ...alignX, top: `${-SIZE_LABEL.height - SIZE_LABEL.gap}px` }
  if (s.height >= 64 && s.width >= SIZE_LABEL.width + 12) return { ...alignX, top: '6px' }
  return s.x + s.width + SIZE_LABEL.gap + SIZE_LABEL.width <= d.x + d.width
    ? { left: `calc(100% + ${SIZE_LABEL.gap}px)`, right: 'auto', top: '0' }
    : { left: 'auto', right: `calc(100% + ${SIZE_LABEL.gap}px)`, top: '0' }
})
// 选区（含上方尺寸标签）压到左上角按钮条时，按钮条让到右上角；两侧都被占用时保持原位。
const modebar = ref<HTMLElement | null>(null)
const modebarRight = computed(() => {
  const s = selection.value
  if (!s) return false
  const d = activeDisplay.value
  const width = (modebar.value?.offsetWidth ?? 320) + 28
  const height = (modebar.value?.offsetHeight ?? 36) + 28
  const top = s.y - SIZE_LABEL.height - SIZE_LABEL.gap
  if (top >= d.y + height) return false
  const hitsLeft = s.x < d.x + width
  const hitsRight = s.x + s.width > d.x + d.width - width
  return hitsLeft && !hitsRight
})
let off: (() => void) | undefined
let offClosed: (() => void) | undefined
// current() 的旧响应或迟到 ready 事件不能让已关闭的大图重新驻留。
let closedThroughId = 0
let offScroll: (() => void) | undefined
let offPin: (() => void) | undefined
let disposed = false
// 识别面板只属于当前截图会话；选区、标注和录屏入口分别维护自己的状态与守卫。
const { qrResult, ocrResult, ocrDraft, ocrMode, translationTrigger, ocrTrigger, closeOcr, recognizeText, recognizeQr } = useCaptureRecognition({
  frame, pixels, busy, error,
  blocked: () => editor.pending || editor.interacting,
  gestureActive: () => !!gesture.value,
  annotations: () => editor.history.present,
  releaseFrame,
})
// 识别工作区左侧显示截图选区原图；OCR 坐标按选区像素映射到整屏图上。
const ocrImage = computed(() => frame.value && pixels.value && imageUrl.value ? { url: imageUrl.value, width: frame.value.width, height: frame.value.height, crop: { ...pixels.value } } : null)
// 窗口按截图原尺寸加文字区计算，打开时取当前屏幕可用区域。
const ocrWindow = computed(() => ocrWindowSize(ocrImage.value, globalThis.devicePixelRatio ?? 1, { width: globalThis.screen?.availWidth ?? 1280, height: globalThis.screen?.availHeight ?? 800 }))
// 录屏准备条与截图选区共用当前帧，但能力查询、格式偏好和接管清理独立维护。
const {
  recordingTrigger, recordingConfirmTrigger, recordingSource,
  recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps,
  recordingCountdown, captureCursor, recordingEffects, recordingSystemAudio, recordingMicrophone,
  recordingCapabilities, recordingCapabilityError, hideMainWindow, checkingRecordingCapabilities,
  reloadPreferences, loadRecordingCapabilities, enterRecordingMode, leaveRecordingMode,
  chooseRecordingSource, selectedRecordingRegion, startRecording,
} = useCaptureRecording({
  frame, pixels, valid, busy, error, annotating, marked, activeDisplay, selection, mode: recordingMode,
  gestureActive: () => !!gesture.value,
  editorBlocked: () => editor.pending || editor.interacting,
  annotations: () => editor.history.present,
  releaseFrame,
  clearTarget: () => { selectedTarget.value = null },
  disposed: () => disposed,
  liveRecordingId, liveAnnotations,
})
const toolbar = ref<HTMLElement | null>(null)
const { style: floatingToolbarStyle, start: floatToolbar, reset: resetToolbarPosition } = useFloatingToolbar(toolbar)
const toolbarSize = ref({ width: 800, height: 46 })
// 按实际尺寸避让屏幕边缘；窄屏允许横向滚动，保留字号和按钮命中区域。
watch(toolbar, (element, _, cleanup) => {
  if (!element) return
  const observer = new ResizeObserver(() => {
    // 拖动选区时用 display:none 隐藏工具栏，不能用零尺寸覆盖最后一次有效测量。
    if (element.offsetWidth > 0 && element.offsetHeight > 0) toolbarSize.value = { width: element.offsetWidth, height: element.offsetHeight }
  })
  observer.observe(element)
  cleanup(() => observer.disconnect())
})
const toolbarStyle = shallowRef<ReturnType<typeof annotationToolbarPosition> | Record<string, never>>({})
// 绘制和调整范围期间锁住工具栏位置；尺寸观察不应让底部工具来回跳动。
watch([selection, activeDisplay, toolbarSize, () => editor.interacting, () => editor.pending, gesture], () => {
  if (selection.value && !editor.interacting && !editor.pending && !gesture.value) {
    toolbarStyle.value = annotationToolbarPosition(selection.value, activeDisplay.value, toolbarSize.value)
  }
}, { immediate: true })
// 所有图形保存在原图坐标系内；选区只负责裁剪和限制绘制区域。
function chooseTool(tool: AnnotationTool) { if (busy.value || editor.pending || editor.interacting) return; editor.setTool(tool); annotating.value = true }
function selectionDown(e: PointerEvent) { if (!annotating.value || editor.tool === 'edit') down(e, 'move') }
function receiveScroll(value: ScrollView) {
  const next = newerScroll(scroll.value, value, frame.value?.id)
  if (next && !next.collecting) scrollClosed()
  else scroll.value = next
}
/** 只在后端确认结束后调用；保存取消或失败时保留标注及撤销栈。 */
function releaseFrame(id = frame.value?.id) {
  if (id !== undefined) closedThroughId = Math.max(closedThroughId, id)
  // 旧导出响应可能晚于新截图 ready 到达，不能清掉新会话。
  if (frame.value && frame.value.id !== id) return
  frame.value = null
  editor.reset()
  selection.value = null; gesture.value = null; pointer.value = null
  imageReady.value = false; selectedTarget.value = null; annotating.value = false; recordingMode.value = false
  qrResult.value = null; ocrResult.value = null
  ocrDraft.value = ''
}
function scrollClosed() { scroll.value = null; releaseFrame() }
async function copyColor(format = colorFormat.value) {
  if (!frame.value || !pointer.value || !imageReady.value || busy.value || editor.pending || editor.interacting) return
  const pixel = magnifierSample(pointer.value, viewport.value, frame.value).pixel
  busy.value = true; error.value = ''
  try { await api.copyColor(frame.value.id, pixel, format); colorCopied.value = true }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
let liveSelection: { rect: Rect | null; annotations: Annotation[] } | null = null
function receive(value: CaptureFrame, fromReplay = false) {
  if (disposed || value.id <= closedThroughId || (frame.value && value.id < frame.value.id)) return
  if (!fromReplay) liveSelection = null
  reloadPreferences()
  liveRecordingId.value = null
  busy.value = false; reselect(); windowsOnly.value = false; scroll.value = null; imageReady.value = false; recordingMode.value = !!value.recording; frame.value = value
  if (value.recording) void loadRecordingCapabilities()
  if (value.replay) {
    selection.value = fromPixels(value.replay.rect, value, viewport.value)
    editor.reset(value.replay.annotations)
  }
}
/** 回放替换整个会话；失败保留现场，不会清空选区或当前标注。 */
async function replay(direction: 'previous' | 'next') {
  if (!frame.value || blocked.value || recordingMode.value || scroll.value || liveRecordingId.value !== null) return
  const id = frame.value.id
  busy.value = true; error.value = ''
  try {
    const live = !frame.value.replay ? { rect: pixels.value ? { ...pixels.value } : null, annotations: JSON.parse(JSON.stringify(editor.history.present)) } : null
    const restored = await api.replayCapture(id, direction)
    if (!disposed && frame.value?.id === id && restored.id !== id) {
      if (restored.replay && live) liveSelection = live
      receive(restored, true)
      if (!restored.replay && liveSelection) {
        selection.value = liveSelection.rect ? fromPixels(liveSelection.rect, restored, viewport.value) : null
        editor.reset(liveSelection.annotations)
        liveSelection = null
      }
    }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** load 只表示资源可用；先解码并等待 Vue 更新与绘制机会，再显示原生蒙层。 */
async function loaded(event: Event) {
  const current = frame.value
  const image = event.target as HTMLImageElement
  if (!current || image !== screenImage.value) return
  const url = imageUrl.value
  const isCurrent = () => !disposed && frame.value?.id === current.id && imageUrl.value === url && screenImage.value === image
  try {
    await presentCaptureImage({ image, receivedAt: imageReceivedAt, isCurrent,
      decoded: () => { imageReady.value = true }, decodeFailed: imageFailed,
      ready: () => api.ready(current.id, current.previewRevision),
      painted: paint => api.capturePainted(current.id, current.previewRevision, paint),
    })
  }
  catch (e) { if (isCurrent()) error.value = String(e) }
}
function key(e: KeyboardEvent) {
  if (e.defaultPrevented || e.isComposing || e.keyCode === 229) return
  if (liveRecordingId.value !== null) return
  if (qrResult.value || ocrResult.value !== null) {
    // 未落在面板内的键盘事件由窗口兜底；同一次 Esc 不继续取消底层截图。
    handleEscape(e, () => {
      if (ocrResult.value !== null) closeOcr()
      else qrResult.value = null
    })
    return
  }
  if (scroll.value) return
  // 行内设置默认可见，不作为模态层消费 Esc；真正的下拉菜单自行关闭当前层。
  // 先退出窗口，再判断输入框和标注快捷键，避免 Esc 只取消文字或笔画。
  if (handleEscape(e, () => { void cancel() })) return
  if ((e.target as HTMLElement).closest('.recording-inline-options')) return
  if ((e.target as HTMLElement).closest('input, textarea, select')) return
  if (!e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && !e.repeat && [',', '.'].includes(e.key)
    && !frame.value?.translation && !annotating.value && !qrResult.value && !ocrResult.value) {
    e.preventDefault(); void replay(e.key === ',' ? 'previous' : 'next'); return
  }
  if (e.key === 'Alt') windowsOnly.value = true
  // 框选前用 Tab 逐级选择按钮、面板或整窗；已选区和表单保留正常焦点导航。
  if (e.key === 'Tab' && !e.ctrlKey && !e.metaKey && !e.altKey && !selection.value && !gesture.value
    && !busy.value && hoverCandidates.value.length > 1) {
    e.preventDefault()
    hoverLevel.value = (hoverLevel.value + (e.shiftKey ? -1 : 1) + hoverCandidates.value.length) % hoverCandidates.value.length
    return
  }
  const command = e.ctrlKey || e.metaKey
  if (annotating.value && editor.key(e)) return
  // 录屏模式拦截截图导出快捷键，避免 Enter / Ctrl+C 意外复制静态图片。
  if (recordingMode.value) {
    if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'BUTTON') { e.preventDefault(); if (!e.repeat) void startRecording(); return }
    if (command && ['c', 's', 'p'].includes(e.key.toLowerCase())) { e.preventDefault(); return }
  }
  if (frame.value?.translation && e.key === 'Enter' && !e.repeat) { e.preventDefault(); void recognizeText(true); return }
  // 取色快捷键仅在放大镜可用时生效；复制后保留选区，修饰键组合仍用于截图操作。
  if (showMagnifier.value && !gesture.value && !command && !e.altKey && !e.repeat) {
    if (e.key === 'Shift') { e.preventDefault(); colorFormat.value = colorFormat.value === 'rgb' ? 'hex' : 'rgb'; colorCopied.value = false; return }
    if (e.key.toLowerCase() === 'c') { e.preventDefault(); void copyColor(); return }
    if (e.key.toLowerCase() === 'p') { e.preventDefault(); relativeCoordinates.value = !relativeCoordinates.value; return }
  }
  // 保留按钮的 Enter / 空格行为，恢复选区后按钮仍聚焦也能使用上方的取色快捷键。
  const target = e.target as HTMLElement
  if (['INPUT', 'SELECT', 'BUTTON'].includes(target.tagName) && e.key !== 'Escape' && !command) return
  if (!command && !e.altKey && !e.repeat && e.key.toLowerCase() === 'r') { e.preventDefault(); restoreSelection(); return }
  // 方向键允许长按连续微调；绘制、导出或已有标注时不改变裁剪范围。
  const directions: Record<string, Point> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } }
  const direction = directions[e.key]
  if (direction && !command) {
    e.preventDefault()
    nudge(direction, e.shiftKey ? 10 : 1, e.altKey)
    return
  }
  if (e.repeat) return
  if (e.key === 'Escape') { e.preventDefault(); void cancel() }
  else if (e.key === 'Enter' || (command && e.key.toLowerCase() === 'c')) { e.preventDefault(); void finish('copy') }
  else if (command && e.key.toLowerCase() === 's') { e.preventDefault(); void finish('save') }
  else if (command && e.key.toLowerCase() === 'p') { e.preventDefault(); void finish('pin') }
  else if (command && e.key.toLowerCase() === 'a') { e.preventDefault(); selectAll() }
  else if (command && e.key.toLowerCase() === 'z') { e.preventDefault(); void editor.travel(e.shiftKey ? 'redo' : 'undo') }
  else if (command && e.key.toLowerCase() === 'y') { e.preventDefault(); void editor.travel('redo') }
}
function keyUp(e: KeyboardEvent) { if (e.key === 'Alt') windowsOnly.value = false }
function resized() { viewport.value = { width: innerWidth, height: innerHeight }; if (!scroll.value && liveRecordingId.value === null) reselect() }
async function imageFailed() {
  error.value = '截图预览加载失败，请重新截图。'
  await cancel()
}
onMounted(async () => {
  window.addEventListener('keydown', key); window.addEventListener('keyup', keyUp); window.addEventListener('resize', resized)
  try {
    offClosed = await onCaptureClosed(id => {
      closedThroughId = Math.max(closedThroughId, id)
      if (frame.value?.id === id) { scroll.value = null; releaseFrame() }
    }, 'capture')
    if (disposed) { offClosed(); return }
    off = await onFrame(receive)
    if (disposed) { off(); return }
    offScroll = await onScroll(receiveScroll)
    if (disposed) { offScroll(); return }
    offPin = await onCapturePin(id => {
      // 复用工具栏的导出校验，编辑/拖动/滚动截图期间不提交，也不回退剪贴板。
      if (frame.value?.id === id && !scroll.value && !recordingMode.value) void finish('pin')
    })
    if (disposed) { offPin(); return }
    const current = await api.current()
    if (current && (!frame.value || current.id > frame.value.id)) receive(current)
    const currentScroll = await api.currentScroll()
    if (currentScroll && !disposed) receiveScroll(currentScroll)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { releaseFrame(); disposed = true; offClosed?.(); off?.(); offScroll?.(); offPin?.(); window.removeEventListener('keydown', key); window.removeEventListener('keyup', keyUp); window.removeEventListener('resize', resized) })
</script>

<template>
  <RecordingAnnotations v-if="liveRecordingId !== null" ref="liveAnnotations" :session-id="liveRecordingId" :initial-editor="editor" @closed="liveRecordingId = null" />
  <div v-else-if="scroll" />
  <div v-else ref="captureRoot" class="capture-overlay" @pointerdown="down($event)" @pointermove.capture="drag" @pointerleave="pointer = null" @pointerup="up" @pointercancel="abortGesture(); pointer = null" @contextmenu.prevent="cancel">
    <img v-if="frame" :key="frame.id" ref="screenImage" class="screen-image" :src="imageUrl" crossorigin="anonymous" :alt='t("当前屏幕截图")' draggable="false" @load="loaded" @error="imageFailed" />
    <!-- 截图和录屏共用选区入口：开始框选前保持原图，仅在形成选区后遮暗外围。 -->
    <div v-if="!selection" class="capture-tip" :style="{ left: `${activeDisplay.x + activeDisplay.width / 2}px`, top: `${activeDisplay.y + 64}px`, maxWidth: `${activeDisplay.width - 32}px`, whiteSpace: 'normal' }">{{ recordingMode ? t("拖动选择录屏范围") : frame?.targets.length ? t("点击吸附 · Tab 切换层级 · 拖动框选 · Alt 选窗口 · Shift 自由框选") : t("拖动框选屏幕") }} <span>{{ recordingMode ? t("Esc 退出 · Ctrl / ⌘ + A 全屏 · R 最近选区") : t("C 取色 · Esc 取消 · Ctrl / ⌘ + A 全屏 · R 最近选区") }}</span></div>
    <div v-if="frame" ref="modebar" class="capture-modebar" :style="modebarRight ? { left: `${activeDisplay.x + activeDisplay.width - 16}px`, top: `${activeDisplay.y + 16}px`, transform: 'translateX(-100%)' } : { left: `${activeDisplay.x + 16}px`, top: `${activeDisplay.y + 16}px` }" @pointerdown.stop @contextmenu.stop.prevent>
      <template v-if="!recordingMode">
        <button :disabled="busy || editor.pending || editor.interacting || !!gesture" :title="t('上一张截图（,）')" @click="replay('previous')">{{ t('上一张 ,') }}</button>
        <span v-if="frame.replay">{{ t('截图历史 {0}/{1}', { 0: frame.replay.index, 1: frame.replay.total }) }}</span>
        <button :disabled="!frame.replay || busy || editor.pending || editor.interacting || !!gesture" :title="t('下一张截图（.），最后一张后返回现场')" @click="replay('next')">{{ t('下一张 .') }}</button>
      </template>
      <button :disabled="!recentRect || busy || marked || editor.pending || editor.interacting" :title="recentRect ? t('恢复上次成功复制、保存或贴图的选区（R）') : t('暂无最近选区，或当前屏幕分辨率已变化')" @click="restoreSelection"><Scan :size="15" />{{ t("最近选区 R") }}</button>
      <span v-if="recordingMode && checkingRecordingCapabilities" role="status">{{ t('正在检查录屏能力…') }}</span>
      <span v-else-if="recordingMode && (recordingCapabilityError || recordingCapabilities?.available === false)" role="alert">{{ te(recordingCapabilityError || recordingCapabilities?.reason) || t('没有可用的录制编码器。') }} <button :disabled="busy" :aria-label="t('重新读取')" @click="loadRecordingCapabilities(true)">{{ t('重新读取') }}</button></span>
    </div>
    <div v-if="hoverTarget !== null" class="snap-target" :style="hoverStyle" />
    <div v-if="selection" class="selection" :class="{ 'annotation-active': annotating }" :style="{ ...boxStyle, boxShadow: valid ? undefined : 'none' }" @pointerdown.stop="selectionDown">
      <span class="size-label" :style="sizeLabelStyle">{{ pixels?.width }} × {{ pixels?.height }} px · ({{ pixels?.x }}, {{ pixels?.y }})</span>
      <template v-if="!editor.interacting && !editor.pending && !busy">
        <span v-for="edge in ['top', 'right', 'bottom', 'left']" :key="edge" :class="['selection-drag-edge', edge]" @pointerdown.stop="down($event, 'move')" />
        <span v-for="handle in handles" :key="handle" :class="['resize-handle', handle]" @pointerdown.stop="down($event, handle)" />
      </template>
    </div>
    <AnnotationWorkspace :editor="editor" :width="frame?.width ?? 1" :height="frame?.height ?? 1" :bounds="pixels ?? { x: 0, y: 0, width: 1, height: 1 }" :surface-visible="!!frame && !!pixels" :toolbar-visible="!!valid && !gesture" :enabled="annotating && !busy && !gesture" :active="annotating" :disabled="busy || !valid || !!gesture" :toolbar-style="[toolbarStyle, floatingToolbarStyle]" @background-pointerdown="down($event, 'move')" @toolbar-element="toolbar = $event" @tool="chooseTool" @float="floatToolbar" @reset-position="resetToolbarPosition">

        <template #actions>
          <button v-if="!recordingMode && selectedTarget !== null && frame?.targets[selectedTarget]?.kind === 'window'" :disabled="busy || marked || editor.pending || editor.interacting" :title='t("原生窗口截图")' :aria-label='t("原生窗口截图")' @click="captureNative"><AppWindow :size="16" /></button>
          <button :disabled="busy || !!gesture" :title='t("清空标注并重新框选")' :aria-label='t("清空标注并重新框选")' @click="reselect"><RotateCcw :size="16" /></button>
          <button :disabled="busy || !!gesture || marked" :title='t("选择整个屏幕 Ctrl/⌘+A")' :aria-label='t("选择整个屏幕")' @click="selectAll"><Monitor :size="16" /></button>
          <button v-if="!recordingMode && !frame?.replay" :disabled="busy || !!gesture || marked" :title='t("滚动截图：框选正文后慢慢滚动")' :aria-label='t("开始滚动截图")' @click="startScroll"><ScrollCaptureIcon :size="16" /></button>
          <button v-if="!frame?.replay" ref="recordingTrigger" class="recording-mode-trigger" :disabled="busy || !!gesture || editor.pending || editor.interacting" :title="t(recordingMode ? '返回截图' : '切换到录屏')" :aria-label="t(recordingMode ? '返回截图' : '展开录屏工具栏')" :aria-pressed="recordingMode" :aria-expanded="recordingMode" @click="recordingMode ? leaveRecordingMode() : enterRecordingMode()"><Video :size="16" /></button>
        <template v-if="!recordingMode">
        <button :disabled="busy || !!gesture" :title='t("贴图 Ctrl/⌘+P")' :aria-label='t("贴图")' @click="finish('pin')"><Pin :size="16" /></button>
        <button ref="ocrTrigger" :disabled="busy || !!gesture" :title='t("识别文字")' :aria-label='t("识别文字")' aria-haspopup="dialog" :aria-expanded="ocrResult !== null" @click="recognizeText(false)"><ScanText :size="16" /></button>
        <button ref="translationTrigger" :disabled="busy || !!gesture" :title='t("截图翻译")' :aria-label='t("截图翻译")' @click="recognizeText(true)"><Languages :size="16" /></button>
        <button :disabled="busy || !!gesture" :title='t("识别二维码")' :aria-label='t("识别二维码")' aria-haspopup="dialog" :aria-expanded="!!qrResult" @click="recognizeQr"><QrCode :size="16" /></button>
        <button :disabled="busy || !!gesture" :title='t("保存图片 Ctrl/⌘+S")' :aria-label='t("保存图片")' @click="finish('save')"><Save :size="16" /></button>
        <button class="copy-button" :disabled="busy || !!gesture" :title="busy ? t('处理中…') : t('复制图片 Enter')" :aria-label="busy ? t('处理中') : t('复制图片')" :aria-busy="busy" @click="finish('copy')"><Copy :size="16" /></button>
        </template>
        <button :disabled="busy || editor.pending" :title="recordingMode ? t('取消录屏') : t('取消截图')" :aria-label='t("取消截图")' @click="cancel"><X :size="16" /></button>
        </template>
      <template #footer>
      <div v-if="recordingMode" class="recording-ready-row" role="toolbar" :aria-label='t("录屏准备工具栏")'>
        <button class="recording-grip" :disabled="busy" :title='t("拖动工具栏；双击恢复位置")' :aria-label='t("拖动录屏工具栏")' @pointerdown="floatToolbar" @dblclick="resetToolbarPosition"><GripVertical :size="16" /></button>
        <button :disabled="busy" :title='t("返回截图")' :aria-label='t("返回截图")' @click="leaveRecordingMode"><ArrowLeft :size="16" /></button>
        <div class="recording-control-group" role="group" :aria-label="t('录制范围')">
        <RecordingSourceSelect :value="recordingSource" :disabled="busy || marked || editor.interacting" @select="chooseRecordingSource" />
        <SelectControl v-model="recordingFormat" :disabled="busy || !recordingCapabilities?.available" :aria-label="t('录屏格式')" :options="(recordingCapabilities?.formats ?? []).map(value => ({ value, label: value.toUpperCase() }))" />
        <SelectControl v-model="recordingFps" :disabled="busy" :aria-label="t('录屏帧率')" :options="(recordingFormat === 'gif' ? [10, 20] : [30, 60]).map(value => ({ value, label: `${value} FPS` }))" />
        </div>
        <div class="recording-control-group recording-toggles" role="group" :aria-label="t('录屏选项')">
        <button :disabled="busy" :aria-pressed="captureCursor" :title="`${t('录制鼠标指针')} · ${t(captureCursor ? '已启用' : '已关闭')}`" :aria-label='t("录制鼠标指针")' @click="captureCursor = !captureCursor"><MousePointer2 :size="16" /></button>
        <template v-if="recordingFormat === 'mp4'">
          <button :disabled="busy || !recordingCapabilities?.systemAudio" :aria-pressed="recordingSystemAudio" :title="recordingCapabilities?.systemAudio ? `${t('录制系统声音')} · ${t(recordingSystemAudio ? '已启用' : '已关闭')}` : t('系统声音录制不可用')" :aria-label='t("录制系统声音")' @click="recordingSystemAudio = !recordingSystemAudio"><component :is="recordingSystemAudio ? Volume2 : VolumeX" :size="16" /></button>
          <button :disabled="busy || !recordingCapabilities?.microphone" :aria-pressed="recordingMicrophone" :title="recordingCapabilities?.microphone ? `${t('录制麦克风')} · ${t(recordingMicrophone ? '已启用' : '已关闭')}` : t('麦克风录制不可用')" :aria-label='t("录制麦克风")' @click="recordingMicrophone = !recordingMicrophone"><component :is="recordingMicrophone ? Mic : MicOff" :size="16" /></button>
        </template>
        </div>
        <RecordingSetupMenu id="recording-inline-options" :format="recordingFormat" v-model:encoding="recordingEncoding" v-model:quality="recordingQuality" v-model:mbps="recordingMbps" v-model:countdown="recordingCountdown" v-model:effects="recordingEffects" v-model:hide-main-window="hideMainWindow" :busy="busy" />
        <div class="recording-final-actions">
        <button :disabled="busy" :title='t("重新选择录屏范围")' :aria-label='t("重新选择录屏范围")' @click="reselect"><RotateCcw :size="16" /></button>
        <button ref="recordingConfirmTrigger" class="recording-confirm" :disabled="!valid || busy || editor.pending || editor.interacting || !recordingCapabilities?.available" :title="recordingCapabilities?.reason ?? (recordingCapabilityError || (recordingCapabilities ? t('确认范围，进入录制控制条') : t('正在检查录屏能力…')))" @click="startRecording"><Check :size="15" />{{ busy ? t("准备中…") : t("确认范围") }}</button>
        <button :disabled="busy" :title='t("取消录屏")' :aria-label='t("取消录屏")' @click="cancel"><X :size="16" /></button>
        </div>
      </div>
      </template>
    </AnnotationWorkspace>
    <CaptureMagnifier v-if="showMagnifier && screenImage && pointer && frame" :image="screenImage" :point="pointer" :viewport="viewport" :image-size="frame" :format="colorFormat" :relative="relativeCoordinates" :origin="pixels" :copied="colorCopied" :bounds="pointerDisplay" />
    <DetachedWindow :open="ocrResult !== null" :title="ocrMode === 'translate' ? t('截图翻译') : t('文字识别结果')" :width="ocrWindow.width" :height="ocrWindow.height" @close="closeOcr">
      <OcrWorkspace v-if="ocrResult" v-model:text="ocrDraft" :result="ocrResult" :image="ocrImage" :mode="ocrMode" :error="error" @close="closeOcr" />
    </DetachedWindow>
    <DetachedWindow :open="!!qrResult" :title="t('二维码识别结果')" :width="480" :height="420" @close="qrResult = null">
      <div v-if="qrResult" class="qr-dialog">
        <QrResultCard :capture-result="qrResult" @close="qrResult = null" @opened="cancel" />
      </div>
    </DetachedWindow>
    <div v-if="error" class="overlay-error" :style="{ left: `${activeDisplay.x + activeDisplay.width / 2}px`, bottom: 'auto', top: `${activeDisplay.y + 16}px`, maxWidth: `${activeDisplay.width - 32}px` }" role="alert" @pointerdown.stop>{{ te(error) }}</div>
  </div>
</template>

<style scoped>
.selection { z-index:3; }
.selection.annotation-active { pointer-events:none; }
.selection .resize-handle { z-index:2; pointer-events:auto; }
.selection-drag-edge { position:absolute; pointer-events:auto; cursor:move; }
.selection-drag-edge.top, .selection-drag-edge.bottom { left:6px; right:6px; height:8px; }
.selection-drag-edge.top { top:-4px; }
.selection-drag-edge.bottom { bottom:-4px; }
.selection-drag-edge.left, .selection-drag-edge.right { top:6px; bottom:6px; width:8px; }
.selection-drag-edge.left { left:-4px; }
.selection-drag-edge.right { right:-4px; }

.capture-overlay .recording-mode-trigger[aria-pressed=true] { color:#2563eb; background:#dbeafe; }
/* 常用操作按用途分组；窄屏按组换行，确认和取消保持在同一组。 */
.recording-ready-row { border-top:1px solid #e1e4e8; padding-top:6px; display:flex; flex-wrap:wrap; align-items:center; gap:6px; min-height:38px; max-width:100%; font-size:13px; }
.recording-control-group { display:inline-flex; flex:none; align-items:center; gap:3px; padding:2px; border:1px solid #e1e4e8; border-radius:5px; background:#f6f7f8; }
.recording-ready-row select { height:28px; padding:2px 5px; min-width:0; background:transparent; color:inherit; border:0; border-radius:4px; font:inherit; cursor:pointer; }
.recording-ready-row select:hover:enabled { background:#e9edf2; }
.recording-ready-row :deep(.recording-source-trigger) { padding:3px 6px; }
.recording-toggles button[aria-pressed=true] { color:#2563eb; background:#dbeafe; box-shadow:inset 0 0 0 1px #bfdbfe; }
.recording-toggles button[aria-pressed=false] { color:#57606a; }
.recording-final-actions { display:inline-flex; align-items:center; gap:4px; margin-left:auto; }
.recording-ready-row .recording-confirm { min-width:88px; padding:3px 10px; background:#2563eb; color:white; font-weight:600; }
.recording-ready-row .recording-confirm:hover:enabled { background:#1d4ed8; }
.recording-grip { cursor:grab; touch-action:none; }
.recording-ready-row :focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
:global([data-theme=dark]) .recording-control-group { background:#272e37; border-color:#444c56; }
:global([data-theme=dark]) .recording-ready-row { border-color:#444c56; }
:global([data-theme=dark]) .recording-toggles button[aria-pressed=true] { color:#bfdbfe; background:#263f60; box-shadow:inset 0 0 0 1px #45658b; }
:global([data-theme=dark]) .recording-toggles button[aria-pressed=false] { color:#a8b3c2; }
:global([data-theme=dark]) .recording-ready-row select:hover:enabled { background:#303f56; }
.qr-dialog { width: 100%; height: 100%; overflow-y: auto; color: #20232b; }

.capture-modebar { position: absolute; top: 8px; left: 8px; z-index: 4; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; max-width: calc(100vw - 16px); border-radius: 6px; padding: 6px 8px; background: #242330ed; color: white; font-size: 12px; cursor: default; }
.capture-modebar button { display: flex; align-items: center; gap: 5px; padding: 3px; }
.capture-modebar button:disabled { opacity: .5; }
.snap-target { position: absolute; outline: 2px solid #60a5fa; pointer-events: none; }
</style>
