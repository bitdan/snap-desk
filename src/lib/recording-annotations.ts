import type { Annotation } from './annotations'
import { shapeBounds } from './annotations'
import { prepareRaster } from './annotation-raster'

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
