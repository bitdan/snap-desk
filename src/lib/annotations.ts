import { clamp, type Point, type Rect, type Size } from './selection'

// 标注始终使用原图物理像素坐标，预览和 Rust 导出共用数据，避免 DPI 缩放错位。
export interface Annotation {
  kind: 'rectangle' | 'arrow' | 'ellipse' | 'line'
  start: Point
  end: Point
  color: string
  width: number
}
export interface History { present: Annotation[]; future: Annotation[] }
export const emptyHistory = (): History => ({ present: [], future: [] })
/** 将鼠标的 CSS 坐标转为原图像素，并限制在当前裁剪选区。 */
export function imagePoint(point: Point, viewport: Size, image: Size, crop: Rect): Point {
  return { x: clamp(point.x * image.width / viewport.width, crop.x, crop.x + crop.width), y: clamp(point.y * image.height / viewport.height, crop.y, crop.y + crop.height) }
}
export const append = (h: History, shape: Annotation): History => ({ present: [...h.present, shape], future: [] })
export function undo(h: History): History {
  const last = h.present.at(-1)
  return last ? { present: h.present.slice(0, -1), future: [...h.future, last] } : h
}
export function redo(h: History): History {
  const last = h.future.at(-1)
  return last ? { present: [...h.present, last], future: h.future.slice(0, -1) } : h
}
// 箭头由主线和两条翼线组成，尺寸公式与 annotations.rs 保持一致。
export function arrowWings(shape: Annotation): [Point, Point] {
  const dx = shape.end.x - shape.start.x, dy = shape.end.y - shape.start.y
  const length = Math.hypot(dx, dy)
  if (!length) return [shape.end, shape.end]
  const size = Math.min(Math.max(10, shape.width * 4), length * 0.45)
  const ux = dx / length, uy = dy / length
  return [
    { x: shape.end.x - ux * size + uy * size * 0.5, y: shape.end.y - uy * size - ux * size * 0.5 },
    { x: shape.end.x - ux * size - uy * size * 0.5, y: shape.end.y - uy * size + ux * size * 0.5 },
  ]
}
export function shapePath(shape: Annotation): string {
  const a = shape.start, b = shape.end
  if (shape.kind === 'rectangle') return `M ${a.x} ${a.y} H ${b.x} V ${b.y} H ${a.x} Z`
  if (shape.kind === 'line') return `M ${a.x} ${a.y} L ${b.x} ${b.y}`
  // 两段半椭圆组成闭合轮廓；取绝对半径，支持从任意方向拖动。
  if (shape.kind === 'ellipse') {
    const rx = Math.abs(b.x - a.x) / 2, ry = Math.abs(b.y - a.y) / 2
    const cx = (a.x + b.x) / 2, cy = (a.y + b.y) / 2
    return `M ${cx - rx} ${cy} A ${rx} ${ry} 0 1 0 ${cx + rx} ${cy} A ${rx} ${ry} 0 1 0 ${cx - rx} ${cy} Z`
  }
  const [left, right] = arrowWings(shape)
  return `M ${a.x} ${a.y} L ${b.x} ${b.y} M ${left.x} ${left.y} L ${b.x} ${b.y} L ${right.x} ${right.y}`
}
