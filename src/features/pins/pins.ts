import type { PinView } from '@/features/pins/types'
import layout from '@/features/pins/pin-layout.json'

/** 保持宽高比，短边至少 20 个逻辑像素；原图不足 20px 时也不能豁免。 */
export function minimumZoom(width: number, height: number, pixelRatio = 1): number {
  return Math.max(0.1, layout.minVisibleSide * pixelRatio / Math.max(1, Math.min(width, height)))
}

/** 原生窗口缩放和 IPC 回包不同步，按当前视口计算边框，不能让百分比尺寸参与网格固有尺寸计算。 */
export function pinDisplaySize(width: number, height: number, zoom: number, viewportWidth: number, viewportHeight: number, expanded: boolean, pixelRatio = 1) {
  // 图片尺寸为物理像素，DOM 为逻辑像素；100% 时一个图片像素对应一个屏幕像素。
  const scale = Math.max(zoom, minimumZoom(width, height, pixelRatio)) / pixelRatio
  const imageWidth = width * scale, imageHeight = height * scale
  const border = layout.border * 2
  const availableWidth = viewportWidth - layout.padding * 2
  const availableHeight = viewportHeight - layout.padding * 2 - layout.gap - (expanded ? layout.expandedHeight : layout.collapsedHeight)
  return {
    imageWidth, imageHeight,
    stageWidth: Math.max(layout.minVisibleSide + border, Math.min(imageWidth + border, availableWidth)),
    stageHeight: Math.max(layout.minVisibleSide + border, Math.min(imageHeight + border, availableHeight)),
  }
}

/** 缩放只改变查看比例，按 10% 步进并消除小数累计误差；导出始终使用原图像素。 */
export function stepZoom(zoom: number, direction: number, minimum = 0.1): number {
  return Math.min(Math.max(4, minimum), Math.max(minimum, Math.round((zoom + Math.sign(direction) * 0.1) * 10) / 10))
}
/** 统一菜单和键盘入口的锁定行为，忙碌期间不叠加原生请求。 */
export function canEditPin(view: PinView | undefined, busy: boolean): boolean {
  return !!view && !view.locked && !busy
}
