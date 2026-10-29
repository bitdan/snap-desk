import type { CaptureDisplay, CaptureTarget } from '@/features/capture/types'
import type { Point, Rect, Size } from '@/shared/geometry/selection'

const contains = (rect: Rect, point: Point) => point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height

/** 按从小到大提供鼠标下的控件/面板层级，始终限制在最上层窗口内。 */
export function targetsAt(targets: CaptureTarget[], point: Point, windowsOnly = false): number[] {
  const parent = targets.findIndex(target => target.kind === 'window' && !target.recordingOnly && contains(target.rect, point))
  if (parent < 0) return []
  if (windowsOnly) return [parent]
  const outer = targets[parent].rect
  const area = (rect: Rect) => rect.width * rect.height
  const same = (a: Rect, b: Rect) => Math.abs(a.x - b.x) <= 2 && Math.abs(a.y - b.y) <= 2
    && Math.abs(a.x + a.width - b.x - b.width) <= 2 && Math.abs(a.y + a.height - b.y - b.height) <= 2
  const candidates = targets.map((target, index) => ({ target, index }))
    .filter(({ target }) => target.kind === 'element' && !target.recordingOnly
      && target.windowIndex === targets[parent].windowIndex && contains(target.rect, point)
      && target.rect.width >= 12 && target.rect.height >= 12
      && target.rect.x >= outer.x && target.rect.y >= outer.y
      && target.rect.x + target.rect.width <= outer.x + outer.width
      && target.rect.y + target.rect.height <= outer.y + outer.height && !same(target.rect, outer))
    .sort((a, b) => area(a.target.rect) - area(b.target.rect))
  const chain: number[] = []
  for (const { target, index } of candidates) {
    if (!chain.some(chosen => same(targets[chosen].rect, target.rect))) chain.push(index)
  }
  return [...chain, parent]
}

/** 默认选择最小有效区域；Alt 直接选择所属窗口。 */
export function targetAt(targets: CaptureTarget[], point: Point, windowsOnly = false): number | null {
  return targetsAt(targets, point, windowsOnly)[0] ?? null
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
