import type { PinView, PinAppearance } from '@/features/pins/types'
import layout from '@/features/pins/pin-layout.json'

/** 与原生窗口定位共用物理像素取整；125% / 150% DPI 下首像素也落在整数屏幕坐标。 */
export function pinImagePadding(pixelRatio = 1): number {
  return Math.round((layout.padding + layout.border) * pixelRatio) / pixelRatio - layout.border
}

/** 保持宽高比，短边至少 20 个逻辑像素；原图不足 20px 时也不能豁免。 */
export function minimumZoom(width: number, height: number, pixelRatio = 1): number {
  return Math.max(0.1, layout.minVisibleSide * pixelRatio / Math.max(1, Math.min(width, height)))
}

/** 工具栏占用的高度（含与图片的间隙），与原生布局一致。 */
export const pinToolbarBand = layout.gap + layout.toolbarHeight

/**
 * 图片边框外框在窗口内的逻辑位置。原生布局给出图片首像素的物理偏移；
 * 尚未布局（0）时按默认内边距处理。
 */
export function pinStageOrigin(view: Pick<PinView, 'imageX' | 'imageY'> | undefined, pixelRatio = 1) {
  const fallback = pinImagePadding(pixelRatio)
  const at = (value: number | undefined) => value && value > 0 ? value / pixelRatio - layout.border : fallback
  return { left: at(view?.imageX), top: at(view?.imageY) }
}

/**
 * 原生窗口缩放和 IPC 回包不同步，按当前视口计算边框，大图在边框内滚动。
 * 工具栏在图片外侧：显示在下方时占用图片下方的一条，在上方时已计入图片的起点。
 */
export function pinDisplaySize(width: number, height: number, zoom: number, viewportWidth: number, viewportHeight: number,
  toolbarBelow: boolean, pixelRatio = 1, origin = pinStageOrigin(undefined, pixelRatio)) {
  // 图片尺寸为物理像素，DOM 为逻辑像素；100% 时一个图片像素对应一个屏幕像素。
  const scale = Math.max(zoom, minimumZoom(width, height, pixelRatio)) / pixelRatio
  const imageWidth = width * scale, imageHeight = height * scale
  const border = layout.border * 2
  const padding = pinImagePadding(pixelRatio)
  const availableWidth = viewportWidth - origin.left - padding
  const availableHeight = viewportHeight - origin.top - padding - (toolbarBelow ? pinToolbarBand : 0)
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
  return !!view && !view.locked && !view.clickThrough && !busy
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
