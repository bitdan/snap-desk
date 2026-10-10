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
// Map CSS edges to captured physical pixels; do not assume devicePixelRatio matches the capture.
export function toPixels(rect: Rect, viewport: Size, image: Size): Rect {
  const x = clamp(Math.floor(rect.x * image.width / viewport.width), 0, image.width)
  const y = clamp(Math.floor(rect.y * image.height / viewport.height), 0, image.height)
  const right = clamp(Math.ceil((rect.x + rect.width) * image.width / viewport.width), x, image.width)
  const bottom = clamp(Math.ceil((rect.y + rect.height) * image.height / viewport.height), y, image.height)
  return { x, y, width: right - x, height: bottom - y }
}
