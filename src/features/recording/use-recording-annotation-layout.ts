import { computed, onMounted, onUnmounted, watch, type Ref, type CSSProperties } from 'vue'
import { useFloatingToolbar } from '@/features/annotations/toolbar-layout'
import { clamp } from '@/shared/geometry/selection'
import type { Rect } from '@/shared/geometry/selection'
import type { RecordingAnnotationView } from './recording-annotations'
import { api } from '@/services/api'

/** CSS 尺寸按整个显示器的实际视口比例转为物理像素，兼容 DPI 和负坐标显示器。 */
export function recordingInputRect(rect: Rect, viewport: { width: number; height: number }, screen: { width: number; height: number }): Rect | null {
  if (![rect.x, rect.y, rect.width, rect.height, viewport.width, viewport.height, screen.width, screen.height].every(Number.isFinite)
    || rect.width <= 0 || rect.height <= 0 || viewport.width <= 0 || viewport.height <= 0) return null
  const x = Math.max(0, Math.floor(rect.x * screen.width / viewport.width))
  const y = Math.max(0, Math.floor(rect.y * screen.height / viewport.height))
  const right = Math.min(screen.width, Math.ceil((rect.x + rect.width) * screen.width / viewport.width))
  const bottom = Math.min(screen.height, Math.ceil((rect.y + rect.height) * screen.height / viewport.height))
  return right > x && bottom > y ? { x, y, width: right - x, height: bottom - y } : null
}

/** 工具栏和 Portal 菜单共同参与原生区域；纯 CSS pointer-events 无法让透明窗口穿透到桌面。 */
export function useRecordingAnnotationLayout(view: Ref<RecordingAnnotationView | null>, toolbar: Ref<HTMLElement | null>, fail: (error: unknown) => void) {
  const floating = useFloatingToolbar(toolbar)
  const style = computed<CSSProperties>(() => {
    const v = view.value, viewport = floating.viewport.value
    if (!v) return {}
    const selection = { x: v.x * viewport.width / v.viewportWidth, y: v.y * viewport.height / v.viewportHeight,
      width: v.width * viewport.width / v.viewportWidth, height: v.height * viewport.height / v.viewportHeight }
    // 两行作为整体定位：录制控制在前，标注工具在后；默认放框下方，空间不足时整体避让。
    const controlsHeight = 48, gap = 8, margin = 8
    const width = Math.min(Math.max(640, floating.size.value.width), viewport.width - margin * 2)
    const height = controlsHeight + gap + floating.size.value.height
    const below = selection.y + selection.height + gap
    const above = selection.y - gap - height
    const groupTop = below + height <= viewport.height - margin ? below : above >= margin ? above
      : clamp(below, margin, Math.max(margin, viewport.height - height - margin))
    const left = clamp(selection.x + selection.width - width, margin, Math.max(margin, viewport.width - width - margin))
    const floatingStyle = floating.style.value
    // 拖动标注栏时为第一行留出空间，原生控制窗随它一起移动。
    const top = floatingStyle.top === undefined ? groupTop + controlsHeight + gap
      : clamp(parseFloat(floatingStyle.top), controlsHeight + gap + margin,
        Math.max(controlsHeight + gap + margin, viewport.height - floating.size.value.height - margin))
    return { ...floatingStyle,
      left: `${floatingStyle.left === undefined ? left : clamp(parseFloat(floatingStyle.left), margin, Math.max(margin, viewport.width - width - margin))}px`,
      top: `${top}px`, maxWidth: `${Math.max(1, viewport.width - margin * 2)}px`,
      maxHeight: `${Math.max(1, viewport.height - controlsHeight - gap - margin * 2)}px`, overflowY: 'auto' }
  })
  let observer: MutationObserver | undefined, resize: ResizeObserver | undefined
  let frame: number | undefined, disposed = false, sending = false, sent = '', generation = 0
  let queued: { id: number; regions: Rect[]; generation: number } | null = null
  async function publish() {
    if (sending) return
    sending = true
    try {
      while (queued && !disposed) {
        const next = queued; queued = null
        const signature = JSON.stringify(next)
        if (signature === sent) continue
        await api.setRecordingAnnotationRegions(next.id, next.regions)
        if (next.generation === generation) sent = signature
      }
    } catch (error) { if (!disposed) fail(error) }
    finally { sending = false }
  }
  function measure() {
    frame = undefined
    const v = view.value
    if (disposed || !v?.editing || !toolbar.value) return
    const nodes = [toolbar.value, ...document.querySelectorAll<HTMLElement>('.annotation-group-menu[data-state="open"]')]
    const regions = nodes.flatMap(node => {
      const rect = node.getBoundingClientRect()
      const region = recordingInputRect({ x: rect.left, y: rect.top, width: rect.width, height: rect.height },
        floating.viewport.value, { width: v.viewportWidth, height: v.viewportHeight })
      return region ? [region] : []
    })
    queued = { id: v.id, regions, generation }; void publish()
  }
  function schedule() { if (frame === undefined && !disposed) frame = requestAnimationFrame(measure) }
  watch([view, toolbar, style], () => {
    resize?.disconnect()
    if (toolbar.value) resize?.observe(toolbar.value)
    schedule()
  }, { flush: 'post' })
  watch([() => view.value?.id, () => view.value?.editing], () => { generation++; sent = ''; queued = null; schedule() })
  watch(() => view.value?.id, () => floating.reset())
  onMounted(() => {
    resize = new ResizeObserver(schedule)
    if (toolbar.value) resize.observe(toolbar.value)
    observer = new MutationObserver(schedule)
    observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['style', 'class', 'data-state'] })
    window.addEventListener('resize', schedule)
    schedule()
  })
  onUnmounted(() => {
    disposed = true; queued = null; observer?.disconnect(); resize?.disconnect()
    if (frame !== undefined) cancelAnimationFrame(frame)
    window.removeEventListener('resize', schedule)
  })
  return { style, start: floating.start, reset: floating.reset }
}
