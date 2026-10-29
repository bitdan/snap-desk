import type { PinView, PinAppearance } from '@/features/pins/types'
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

export function defaultPinAppearance(): PinAppearance { return { shadow: 'auto', cornerRadius: 8, borderColor: '#a1a1aa' } }
export function validatePinAppearance(value: PinAppearance): string | null {
  if (!/^#[0-9a-f]{6}$/i.test(value.borderColor)) return '边框颜色需为六位十六进制颜色。'
  return !['auto', 'soft', 'none'].includes(value.shadow) || !Number.isInteger(value.cornerRadius) || value.cornerRadius < 0 || value.cornerRadius > 24
    ? '贴图圆角需为 0～24 像素。' : null
}
/** 阴影始终留在窗口 12px 透明边距内；自动模式随实际显示尺寸、透明度与焦点变化。 */
export function pinAppearanceStyle(appearance: PinAppearance, width: number, height: number, opacity = 1, focused = false) {
  const shortSide = Math.max(1, Math.min(width, height))
  const radius = Math.min(appearance.cornerRadius, shortSide / 2)
  const borderColor = /^#[0-9a-f]{6}$/i.test(appearance.borderColor) ? appearance.borderColor : defaultPinAppearance().borderColor
  if (appearance.shadow === 'none') return { borderColor, borderRadius: `${radius}px`, boxShadow: 'none' }
  const blur = appearance.shadow === 'auto' ? Math.min(8, Math.max(3, Math.round(shortSide / 32))) : 6
  const alpha = Math.min(0.3, (appearance.shadow === 'auto' ? 0.18 + (focused ? 0.06 : 0) : 0.16) * Math.max(0.2, opacity))
  return { borderColor, borderRadius: `${radius}px`, boxShadow: `0 1px 2px #152b4e14, 0 2px ${blur}px -2px rgba(21, 43, 78, ${alpha.toFixed(3)})` }
}
