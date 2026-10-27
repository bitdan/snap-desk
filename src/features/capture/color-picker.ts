import { clamp, type Point, type Size } from '@/shared/geometry/selection'
import { magnifierSample } from '@/features/capture/magnifier'

/** 展示与原生复制共用 RGB 格式约定；这里只用于放大镜预览。 */
export function colorLabels(r: number, g: number, b: number) {
  const values = [r, g, b].map(value => clamp(Math.round(value), 0, 255))
  return { hex: '#' + values.map(value => value.toString(16).padStart(2, '0')).join('').toUpperCase(), rgb: `rgb(${values.join(', ')})` }
}

/** 方向键每次移动一个原图像素，回到该像素中心，避免分数 DPI 下反复取整漂移。 */
export function nudgeColorPoint(point: Point, delta: Point, viewport: Size, image: Size): Point {
  const { pixel } = magnifierSample(point, viewport, image)
  return { x: (clamp(pixel.x + delta.x, 0, image.width - 1) + .5) * viewport.width / image.width,
    y: (clamp(pixel.y + delta.y, 0, image.height - 1) + .5) * viewport.height / image.height }
}
