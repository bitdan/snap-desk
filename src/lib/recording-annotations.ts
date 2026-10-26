import type { Annotation } from './annotations'
import { shapeBounds } from './annotations'
import { prepareRaster } from './annotation-raster'
import { clamp, type Rect, type Size } from './selection'

/** DOM 是 CSS 像素；原生窗口裁剪采用显示器物理像素，转换后限制在当前显示器内。 */
export function recordingToolbarRegion(rect: Rect, viewport: Size, screen: Size): Rect {
  const sx = screen.width / Math.max(1, viewport.width), sy = screen.height / Math.max(1, viewport.height)
  const x = clamp(Math.floor(rect.x * sx), 0, screen.width - 1)
  const y = clamp(Math.floor(rect.y * sy), 0, screen.height - 1)
  const right = clamp(Math.ceil((rect.x + rect.width) * sx), x + 1, screen.width)
  const bottom = clamp(Math.ceil((rect.y + rect.height) * sy), y + 1, screen.height)
  return { x, y, width: right - x, height: bottom - y }
}

/** 物理像素坐标；浮层按显示器尺寸换算比例，兼容不同 DPI。 */
export interface RecordingAnnotationView {
  id: number; revision: number; width: number; height: number
  viewportWidth: number; viewportHeight: number; x: number; y: number
  editing: boolean; shapes?: Annotation[]
}

/** 实时滤镜只提交范围，原生线程针对每帧处理；文字仍复用截图的字形渲染。 */
export const prepareRecordingRaster: typeof prepareRaster = async (shape, url, crop, preserve) => {
  if (!['mosaic', 'blur', 'magnify'].includes(shape.kind)) return prepareRaster(shape, url, crop, preserve)
  const box = shapeBounds(shape)
  return { ...shape, start: { x: box.x, y: box.y }, end: { x: box.x + box.width, y: box.y + box.height },
    raster: undefined, opacity: shape.kind === 'magnify' ? shape.opacity ?? 1 : 1 }
}
