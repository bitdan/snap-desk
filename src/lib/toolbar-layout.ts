import { computed, onUnmounted, ref, watch, type Ref } from 'vue'
import type { Point, Size } from './selection'

/** 所有位置使用 WebView 的 CSS 像素，不能混入截图的物理像素。 */
export function clampToolbar(point: Point, size: Size, viewport: Size): Point {
  return { x: Math.max(0, Math.min(point.x, viewport.width - size.width)),
    y: Math.max(0, Math.min(point.y, viewport.height - size.height)) }
}

/** 父容器负责默认布局；第一次拖动后改用视口内的固定位置。 */
export function useFloatingToolbar(element: Ref<HTMLElement | null>) {
  const position = ref<Point | null>(null)
  const size = ref<Size>({ width: 0, height: 0 })
  const viewport = ref<Size>({ width: innerWidth, height: innerHeight })
  let drag: { id: number; start: Point; origin: Point; handle: HTMLElement } | null = null
  const bounded = computed(() => position.value && clampToolbar(position.value, size.value, viewport.value))
  const style = computed(() => bounded.value ? { position: 'fixed' as const, left: `${bounded.value.x}px`,
    top: `${bounded.value.y}px`, transform: 'none', margin: '0', height: 'auto', zIndex: 5 } : {})
  watch(element, (node, _, cleanup) => {
    if (!node) return
    const measure = () => { const rect = node.getBoundingClientRect(); size.value = { width: rect.width, height: rect.height } }
    const observer = new ResizeObserver(measure)
    observer.observe(node); measure()
    cleanup(() => { observer.disconnect(); end() })
  }, { immediate: true })
  function resized() { viewport.value = { width: innerWidth, height: innerHeight } }
  function move(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.id) return
    position.value = clampToolbar({ x: drag.origin.x + event.clientX - drag.start.x,
      y: drag.origin.y + event.clientY - drag.start.y }, size.value, viewport.value)
  }
  /** 多指操作时，其他指针抬起不能结束当前手柄的拖动。 */
  function finish(event: PointerEvent) {
    if (event.pointerId !== drag?.id) return
    if (event.type === 'pointerup') move(event)
    end()
  }
  // 取消、失焦、组件卸载都释放捕获与监听，避免工具栏继续跟随鼠标。
  function end() {
    const previous = drag; drag = null
    previous?.handle.removeEventListener('lostpointercapture', end)
    if (previous?.handle.hasPointerCapture(previous.id)) previous.handle.releasePointerCapture(previous.id)
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('pointercancel', finish)
    window.removeEventListener('blur', end)
  }
  function start(event: PointerEvent) {
    if (event.button !== 0 || !element.value || drag) return
    end(); resized()
    const rect = element.value.getBoundingClientRect()
    // ResizeObserver 异步通知；开始拖动时同步量取，避免刚显示/切换样式后的旧尺寸参与限位。
    size.value = { width: rect.width, height: rect.height }
    const handle = event.currentTarget as HTMLElement
    drag = { id: event.pointerId, start: { x: event.clientX, y: event.clientY }, origin: { x: rect.left, y: rect.top }, handle }
    position.value = drag.origin
    handle.setPointerCapture(event.pointerId)
    handle.addEventListener('lostpointercapture', end)
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', finish)
    window.addEventListener('blur', end)
    event.preventDefault()
  }
  function reset() { end(); position.value = null }
  window.addEventListener('resize', resized)
  onUnmounted(() => { end(); window.removeEventListener('resize', resized) })
  return { style, start, reset }
}
