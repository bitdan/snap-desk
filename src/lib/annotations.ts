import { clamp, resize, type Handle, type Point, type Rect, type Size } from './selection'

// 标注始终使用原图物理像素坐标，预览和 Rust 导出共用数据，避免 DPI 缩放错位。
export interface Annotation {
  kind: 'rectangle' | 'arrow' | 'ellipse' | 'line' | 'polyline' | 'pen' | 'highlighter' | 'text' | 'mosaic' | 'blur'
  start: Point
  end: Point
  color: string
  width: number
  opacity?: number
  points?: Point[]
  text?: string
  fontSize?: number
  fontFamily?: 'sans' | 'serif' | 'mono'
  /** 仅文字/局部滤镜携带受限 PNG；Rust 校验格式与大小，不接受 SVG 或文件路径。 */
  raster?: string
}
export interface History { past: Annotation[][]; present: Annotation[]; future: Annotation[][] }
export const emptyHistory = (present: Annotation[] = []): History => ({ past: [], present, future: [] })
/** 将鼠标的 CSS 坐标转为原图像素，并限制在当前裁剪选区。 */
export function imagePoint(point: Point, viewport: Size, image: Size, crop: Rect): Point {
  return { x: clamp(point.x * image.width / viewport.width, crop.x, crop.x + crop.width), y: clamp(point.y * image.height / viewport.height, crop.y, crop.y + crop.height) }
}
/** 每次完成一个操作才保存快照；不可变图形让未修改内容共享引用，最多保留 50 步。 */
export const commit = (h: History, shapes: Annotation[]): History => ({ past: [...h.past, h.present].slice(-50), present: shapes, future: [] })
export const append = (h: History, shape: Annotation): History => commit(h, [...h.present, shape])
export function undo(h: History): History {
  const last = h.past.at(-1)
  return last ? { past: h.past.slice(0, -1), present: last, future: [...h.future, h.present] } : h
}
export function redo(h: History): History {
  const last = h.future.at(-1)
  return last ? { past: [...h.past, h.present], present: last, future: h.future.slice(0, -1) } : h
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
  if (shape.points?.length) return shape.points.map((p, i) => `${i ? 'L' : 'M'} ${p.x} ${p.y}`).join(' ')
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

export function shapeBounds(shape: Annotation): Rect {
  const points = shape.points?.length ? shape.points : [shape.start, shape.end]
  const xs = points.map(p => p.x), ys = points.map(p => p.y)
  return { x: Math.min(...xs), y: Math.min(...ys), width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) }
}
export function mapShape(shape: Annotation, map: (p: Point) => Point): Annotation {
  return { ...shape, start: map(shape.start), end: map(shape.end), ...(shape.points ? { points: shape.points.map(map) } : {}) }
}
export function moveShape(shape: Annotation, delta: Point, crop: Rect): Annotation {
  const box = shapeBounds(shape)
  const dx = clamp(delta.x, crop.x - box.x, crop.x + crop.width - box.x - box.width)
  const dy = clamp(delta.y, crop.y - box.y, crop.y + crop.height - box.y - box.height)
  return mapShape(shape, p => ({ x: p.x + dx, y: p.y + dy }))
}
/** 仿射缩放所有路径点；水平/垂直线通过端点编辑，避免除以零。 */
export function resizeShape(shape: Annotation, handle: Handle | 'start' | 'end', point: Point, crop: Rect): Annotation {
  const p = { x: clamp(point.x, crop.x, crop.x + crop.width), y: clamp(point.y, crop.y, crop.y + crop.height) }
  if (handle === 'start' || handle === 'end') return { ...shape, [handle]: p }
  const box = shapeBounds(shape), next = resize(box, handle, p)
  return mapShape(shape, value => ({ x: next.x + (box.width ? (value.x - box.x) / box.width : 0.5) * next.width, y: next.y + (box.height ? (value.y - box.y) / box.height : 0.5) * next.height }))
}
function segmentDistance(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x, dy = b.y - a.y
  const t = clamp(((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy || 1), 0, 1)
  return Math.hypot(p.x - a.x - t * dx, p.y - a.y - t * dy)
}
/** 从最后绘制的图形开始命中，避免选到被上层图形遮住的对象。 */
export function hitTest(shapes: Annotation[], p: Point, tolerance = 6): number {
  const hit = (s: Annotation) => {
    const b = shapeBounds(s), t = tolerance + s.width / 2
    if (s.points?.length) return s.points.slice(1).some((v, i) => segmentDistance(p, s.points![i], v) <= t)
    if (s.kind === 'line' || s.kind === 'arrow') return segmentDistance(p, s.start, s.end) <= t || (s.kind === 'arrow' && arrowWings(s).some(v => segmentDistance(p, v, s.end) <= t))
    if (s.kind === 'ellipse') {
      const rx = b.width / 2, ry = b.height / 2
      return rx > 0 && ry > 0 && Math.abs(Math.hypot((p.x - b.x - rx) / rx, (p.y - b.y - ry) / ry) - 1) <= t / Math.min(rx, ry)
    }
    const inside = p.x >= b.x - t && p.x <= b.x + b.width + t && p.y >= b.y - t && p.y <= b.y + b.height + t
    return inside && (s.kind !== 'rectangle' || Math.min(Math.abs(p.x - b.x), Math.abs(p.x - b.x - b.width), Math.abs(p.y - b.y), Math.abs(p.y - b.y - b.height)) <= t)
  }
  for (let index = shapes.length - 1; index >= 0; index--) if (hit(shapes[index])) return index
  return -1
}
export const isRaster = (s: Annotation) => ['text', 'mosaic', 'blur'].includes(s.kind)
