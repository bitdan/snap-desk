export interface Point { x: number; y: number }
export interface Rect extends Point { width: number; height: number }
export interface Size { width: number; height: number }
export type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'
export const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value))
export function normalize(a: Point, b: Point): Rect {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) }
}
export function move(rect: Rect, delta: Point, bounds: Size): Rect {
  return { ...rect, x: clamp(rect.x + delta.x, 0, bounds.width - rect.width), y: clamp(rect.y + delta.y, 0, bounds.height - rect.height) }
}
export function resize(rect: Rect, handle: Handle, point: Point): Rect {
  const a = { x: rect.x, y: rect.y }
  const b = { x: rect.x + rect.width, y: rect.y + rect.height }
  if (handle.includes('w')) a.x = point.x
  if (handle.includes('e')) b.x = point.x
  if (handle.includes('n')) a.y = point.y
  if (handle.includes('s')) b.y = point.y
  return normalize(a, b)
}
// 像素坐标往返 CSS 时可能产生极小浮点误差，先吸附整数边缘，避免额外裁出一行/列。
function snapEdge(value: number): number {
  const integer = Math.round(value)
  return Math.abs(value - integer) < 1e-7 ? integer : value
}
// CSS 边缘映射到截图物理像素，不假设 devicePixelRatio 与截图缩放一致。
export function toPixels(rect: Rect, viewport: Size, image: Size): Rect {
  const x = clamp(Math.floor(snapEdge(rect.x * image.width / viewport.width)), 0, image.width)
  const y = clamp(Math.floor(snapEdge(rect.y * image.height / viewport.height)), 0, image.height)
  const right = clamp(Math.ceil(snapEdge((rect.x + rect.width) * image.width / viewport.width)), x, image.width)
  const bottom = clamp(Math.ceil(snapEdge((rect.y + rect.height) * image.height / viewport.height)), y, image.height)
  return { x, y, width: right - x, height: bottom - y }
}

/** 键盘微调以原图像素为单位；缩放模式固定左上角，最小保留 1 × 1 像素。 */
export function nudgeSelection(rect: Rect, delta: Point, viewport: Size, image: Size, resizeMode = false): Rect {
  const pixels = toPixels(rect, viewport, image)
  const next = resizeMode
    ? { ...pixels, width: clamp(pixels.width + delta.x, 1, image.width - pixels.x), height: clamp(pixels.height + delta.y, 1, image.height - pixels.y) }
    : move(pixels, delta, image)
  return { x: next.x * viewport.width / image.width, y: next.y * viewport.height / image.height,
    width: next.width * viewport.width / image.width, height: next.height * viewport.height / image.height }
}
