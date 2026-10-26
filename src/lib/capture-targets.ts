import type { CaptureDisplay, CaptureTarget } from './api'
import type { Point, Rect, Size } from './selection'

const contains = (rect: Rect, point: Point) => point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height

/** 先锁定最上层窗口，再找其最小原生控件，避免选中被遮挡窗口的子控件。 */
export function targetAt(targets: CaptureTarget[], point: Point, windowsOnly = false): number | null {
  const parent = targets.findIndex(target => target.kind === 'window' && !target.recordingOnly && contains(target.rect, point))
  if (parent < 0) return null
  if (windowsOnly) return parent
  let chosen = parent
  targets.forEach((target, index) => {
    if (!target.recordingOnly && target.windowIndex === targets[parent].windowIndex && contains(target.rect, point)
      && target.rect.width * target.rect.height < targets[chosen].rect.width * targets[chosen].rect.height) chosen = index
  })
  return chosen
}

/** CSS 与原图按实际视口比例换算，不使用鼠标所在屏幕的 devicePixelRatio。 */
export function fromPixels(rect: Rect, image: Size, viewport: Size): Rect {
  return { x: rect.x * viewport.width / image.width, y: rect.y * viewport.height / image.height,
    width: rect.width * viewport.width / image.width, height: rect.height * viewport.height / image.height }
}

/** 拖动端点靠近当前窗口/控件边缘时吸附；阈值由调用者换算成物理像素。 */
export function snapPoint(point: Point, target: Rect, threshold: Point): Point {
  const nearest = (value: number, edges: number[], distance: number) => {
    const edge = edges.reduce((a, b) => Math.abs(a - value) <= Math.abs(b - value) ? a : b)
    return Math.abs(edge - value) <= distance ? edge : value
  }
  return { x: nearest(point.x, [target.x, target.x + target.width], threshold.x),
    y: nearest(point.y, [target.y, target.y + target.height], threshold.y) }
}

/** 浮层放在真实显示器内，不落入 L 形布局的虚拟桌面空洞。 */
export function displayBounds(displays: CaptureDisplay[], origin: Point, point: Point, image: Size, viewport: Size): Rect {
  const rects = displays.map(display => ({ ...display, x: display.x - origin.x, y: display.y - origin.y }))
  const selected = rects.find(rect => contains(rect, point)) ?? rects.find(rect => rect.primary) ?? { x: 0, y: 0, ...image }
  return fromPixels(selected, image, viewport)
}
