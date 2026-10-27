import type { OcrResult, PolygonPoint } from '@/features/recognition/ai'

/** 保持引擎阅读顺序；框选、列表勾选与复制共用同一份行 ID。 */
export function selectedOcrText(result: OcrResult, ids: string[]): string {
  const selected = new Set(ids)
  return [...result.lines].sort((a, b) => a.order - b.order)
    .filter(line => selected.has(line.id)).map(line => line.text).join('\n')
}

/** 拒绝异常坐标并限制在图片内；SVG viewBox 负责 CSS 缩放与高 DPI 映射。 */
export function ocrPolygon(points: PolygonPoint[] | null, width: number, height: number): string {
  if (!points || points.length < 3 || width <= 0 || height <= 0
    || points.some(point => !Number.isFinite(point.x) || !Number.isFinite(point.y))) return ''
  return points.map(point => `${Math.max(0, Math.min(width, point.x))},${Math.max(0, Math.min(height, point.y))}`).join(' ')
}
