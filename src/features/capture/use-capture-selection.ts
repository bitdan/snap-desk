//! 截图覆盖层的选区会话：指针、吸附层级、拖动/缩放、最近选区与方向键微调。
//! 坐标统一为覆盖层 CSS 像素；原图裁剪由 pixels 计算，标注编辑通过 late-bound 回调访问。
import { computed, ref, shallowRef, watch, type Ref } from 'vue'
import {
  clamp, move, normalize, resize, toPixels, nudgeSelection,
  type Handle, type Point, type Rect,
} from '@/shared/geometry/selection'
import { targetAt, targetsAt, fromPixels, snapPoint, displayBounds } from './capture-targets'
import { readRecentSelection, restoreRecentSelection, writeRecentSelection } from './recent-selection'
import type { useAnnotations } from '@/features/annotations/use-annotations'
import type { CaptureFrame } from './types'

type AnnotationEditor = ReturnType<typeof useAnnotations>

interface SelectionOptions {
  root: Ref<HTMLElement | null>
  frame: Ref<CaptureFrame | null>
  viewport: Ref<{ width: number; height: number }>
  busy: Ref<boolean>
  error: Ref<string>
  colorCopied: Ref<boolean>
  annotating: Ref<boolean>
  /** 标注编辑器依赖 pixels，故用回调延迟绑定，避免选区与编辑器初始化循环。 */
  getEditor: () => AnnotationEditor
  marked: () => boolean
  recordingMode: Ref<boolean>
  /** 录屏模式与翻译截图需要在框选结束时接力。 */
  onRecordingRegion: () => void
  onAutoTranslate: () => void
  /** 设置了“框选后直接执行”时在这里接力；传入抬起事件以便按住 Ctrl 临时保留工具栏。 */
  onAutoFinish: (event: PointerEvent) => void
}

/** 所有选区状态只描述当前覆盖层会话；关闭或替换截图时由宿主调用 reselect 清理。 */
export function useCaptureSelection({
  root, frame, viewport, busy, error, colorCopied, annotating, getEditor, marked,
  recordingMode, onRecordingRegion, onAutoTranslate, onAutoFinish,
}: SelectionOptions) {
  const selection = ref<Rect | null>(null)
  const selectedTarget = ref<number | null>(null)
  const windowsOnly = ref(false)
  const hoverLevel = ref(0)
  const pointer = ref<Point | null>(null)
  const recentSelection = ref(readRecentSelection())
  const recentRect = computed(() => frame.value ? restoreRecentSelection(recentSelection.value, frame.value, viewport.value) : null)
  // 首次框选完成后工具栏持续挂载；后续拖动只冻结定位，不反复卸载。
  const gesture = shallowRef<{ start: Point; initial: Rect; mode: 'draw' | 'move' | Handle; target: number | null } | null>(null)

  const hoverCandidates = computed(() => {
    if (!frame.value || !pointer.value || selection.value || busy.value || (!recordingMode.value && frame.value.snapEnabled === false)) return []
    return targetsAt(frame.value.targets, { x: pointer.value.x * frame.value.width / viewport.value.width,
      y: pointer.value.y * frame.value.height / viewport.value.height }, windowsOnly.value)
  })
  // 只在命中层级改变时回到最小区域；同一面板内轻微移动不会丢失 Tab 选择。
  watch(() => hoverCandidates.value.join(','), () => { hoverLevel.value = 0 }, { flush: 'sync' })
  const hoverTarget = computed(() => hoverCandidates.value[Math.min(hoverLevel.value, hoverCandidates.value.length - 1)] ?? null)
  const hoverStyle = computed(() => {
    if (!frame.value || hoverTarget.value === null) return {}
    const rect = fromPixels(frame.value.targets[hoverTarget.value].rect, frame.value, viewport.value)
    return { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` }
  })

  const pixels = computed(() => selection.value && frame.value ? toPixels(selection.value, viewport.value, frame.value) : null)
  const valid = computed(() => pixels.value && pixels.value.width > 0 && pixels.value.height > 0)
  const boxStyle = computed(() => selection.value ? { left: `${selection.value.x}px`, top: `${selection.value.y}px`, width: `${selection.value.width}px`, height: `${selection.value.height}px` } : {})
  const activeDisplay = computed(() => {
    if (!frame.value) return { x: 0, y: 0, ...viewport.value }
    const p = selection.value ? { x: selection.value.x + selection.value.width / 2, y: selection.value.y + selection.value.height / 2 } : pointer.value
    return displayBounds(frame.value.displays, { x: frame.value.originX, y: frame.value.originY },
      p ? { x: p.x * frame.value.width / viewport.value.width, y: p.y * frame.value.height / viewport.value.height } : { x: -frame.value.originX, y: -frame.value.originY }, frame.value, viewport.value)
  })
  const pointerDisplay = computed(() => frame.value && pointer.value ? displayBounds(frame.value.displays,
    { x: frame.value.originX, y: frame.value.originY },
    { x: pointer.value.x * frame.value.width / viewport.value.width, y: pointer.value.y * frame.value.height / viewport.value.height }, frame.value, viewport.value) : activeDisplay.value)

  function point(e: PointerEvent): Point { return { x: clamp(e.clientX, 0, viewport.value.width), y: clamp(e.clientY, 0, viewport.value.height) } }

  function down(e: PointerEvent, mode: 'draw' | 'move' | Handle = 'draw') {
    const editor = getEditor()
    if (busy.value || e.button !== 0 || !frame.value) return
    if (editor.pending || editor.interacting) return
    // 已有图层始终使用原图坐标；移动/缩放选区只改变裁剪，重新框选才清空标注。
    if (mode === 'draw') {
      if (marked() || annotating.value) return
      editor.reset()
    }
    const p = point(e)
    pointer.value = p
    const target = mode === 'draw' && !e.shiftKey ? hoverTarget.value : null
    selectedTarget.value = null
    if (mode === 'draw') selection.value = { ...p, width: 0, height: 0 }
    if (!selection.value) return
    gesture.value = { start: p, initial: { ...selection.value }, mode, target }
    ;(root.value ?? e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }

  function drag(e: PointerEvent) {
    colorCopied.value = false
    // 工具栏不需要像素辅助；捕获阶段监听可覆盖选区控制点的事件。
    pointer.value = (e.target as Element).closest('.capture-toolbar, .capture-modebar, .overlay-error') ? null : point(e)
    if (!gesture.value) return
    let p = point(e)
    if (!e.shiftKey && frame.value && (recordingMode.value || frame.value.snapEnabled !== false) && gesture.value.mode !== 'move') {
      const pixel = { x: p.x * frame.value.width / viewport.value.width, y: p.y * frame.value.height / viewport.value.height }
      const index = targetAt(frame.value.targets, pixel, windowsOnly.value)
      if (index !== null) {
        const snapped = snapPoint(pixel, frame.value.targets[index].rect, { x: 6 * frame.value.width / viewport.value.width, y: 6 * frame.value.height / viewport.value.height })
        p = { x: snapped.x * viewport.value.width / frame.value.width, y: snapped.y * viewport.value.height / frame.value.height }
      }
    }
    const { start, initial, mode } = gesture.value
    selection.value = mode === 'draw' ? normalize(start, p) : mode === 'move' ? move(initial, { x: p.x - start.x, y: p.y - start.y }, viewport.value) : resize(initial, mode, p)
  }

  function up(e: PointerEvent) {
    const current = gesture.value
    if (current?.mode === 'draw' && current.target !== null && frame.value && !e.shiftKey
      && Math.hypot(e.clientX - current.start.x, e.clientY - current.start.y) < 4) {
      selectedTarget.value = current.target
      selection.value = fromPixels(frame.value.targets[current.target].rect, frame.value, viewport.value)
    }
    const drawn = gesture.value?.mode === 'draw'
    gesture.value = null
    if (drawn) onRecordingRegion()
    if (drawn) onAutoTranslate()
    if (drawn) onAutoFinish(e)
  }

  function abortGesture() { if (gesture.value) selection.value = gesture.value.initial; gesture.value = null }

  function reselect() {
    if (busy.value) return
    getEditor().reset(); selectedTarget.value = null; selection.value = null; gesture.value = null
    annotating.value = false; error.value = ''; pointer.value = null; colorCopied.value = false
  }

  /** 恢复只替换空白选区，已有标注、文字草稿和进行中的拖动都必须先完成。 */
  function restoreSelection() {
    const editor = getEditor()
    if (!recentRect.value || busy.value || gesture.value || marked() || editor.pending || editor.interacting) return
    const rect = { ...recentRect.value }
    reselect(); selection.value = rect
    onRecordingRegion()
  }

  function selectAll() {
    const editor = getEditor()
    selectedTarget.value = null
    if (busy.value || marked() || editor.interacting) return
    editor.reset(); selection.value = { x: 0, y: 0, ...viewport.value }
    onRecordingRegion()
  }

  /** 方向键允许长按连续微调；绘制、导出或已有标注时不改变裁剪范围。 */
  function nudge(direction: Point, step: number, alt: boolean) {
    const editor = getEditor()
    if (busy.value || gesture.value || editor.interacting || marked() || annotating.value || !valid.value || !selection.value || !frame.value) return
    selectedTarget.value = null
    selection.value = nudgeSelection(selection.value, { x: direction.x * step, y: direction.y * step }, viewport.value, frame.value, alt)
    editor.reset()
  }

  /** 导出成功后记录原图坐标下的最近选区；使用调用前快照，避免迟到的导出结果套用新截图。 */
  function remember(rect: Rect, image: { width: number; height: number; layoutKey?: string }) {
    recentSelection.value = writeRecentSelection(rect, image)
  }

  return {
    selection, selectedTarget, windowsOnly, hoverLevel, pointer, recentSelection, recentRect, gesture,
    hoverCandidates, hoverTarget, hoverStyle, pixels, valid, boxStyle, activeDisplay, pointerDisplay,
    point, down, drag, up, abortGesture, reselect, restoreSelection, selectAll, nudge, remember,
  }
}
