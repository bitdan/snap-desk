import { clamp, type Point, type Size } from './selection'

export const MAGNIFIER_PIXELS = 15
export const MAGNIFIER_ZOOM = 8
export const MAGNIFIER_SIZE = MAGNIFIER_PIXELS * MAGNIFIER_ZOOM

/** 指针对应一个像素而非裁剪边缘；最右/下边界应落在最后一个有效像素。 */
export function magnifierSample(point: Point, viewport: Size, image: Size) {
  const pixel = {
    x: clamp(Math.floor(point.x * image.width / viewport.width), 0, image.width - 1),
    y: clamp(Math.floor(point.y * image.height / viewport.height), 0, image.height - 1),
  }
  const radius = Math.floor(MAGNIFIER_PIXELS / 2)
  const left = pixel.x - radius, top = pixel.y - radius
  const x = Math.max(0, left), y = Math.max(0, top)
  // 靠近边缘时保留空白，不移动采样中心，保证准星一直指向当前像素。
  return { pixel, x, y, width: Math.min(image.width, left + MAGNIFIER_PIXELS) - x,
    height: Math.min(image.height, top + MAGNIFIER_PIXELS) - y,
    dx: (x - left) * MAGNIFIER_ZOOM, dy: (y - top) * MAGNIFIER_ZOOM }
}

/** 优先显示在右下方，空间不足时翻到左/上方；小窗口也不产生负坐标。 */
export function magnifierPosition(point: Point, viewport: Size, panel: Size): Point {
  const place = (p: number, size: number, limit: number) =>
    clamp(p + 24 + size <= limit - 8 ? p + 24 : p - 24 - size, 0, Math.max(0, limit - size))
  return { x: place(point.x, panel.width, viewport.width), y: place(point.y, panel.height, viewport.height) }
}
