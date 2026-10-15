import type { PinView } from './api'
import layout from './pin-layout.json'

/** 保持宽高比，短边至少 20 个逻辑像素；原图不足 20px 时也不能豁免。 */
export function minimumZoom(width: number, height: number): number {
  return Math.max(0.1, layout.minVisibleSide / Math.max(1, Math.min(width, height)))
}

/** 缩放只改变查看比例，按 10% 步进并消除小数累计误差；导出始终使用原图像素。 */
export function stepZoom(zoom: number, direction: number, minimum = 0.1): number {
  return Math.min(Math.max(4, minimum), Math.max(minimum, Math.round((zoom + Math.sign(direction) * 0.1) * 10) / 10))
}
/** 统一菜单和键盘入口的锁定行为，忙碌期间不叠加原生请求。 */
export function canEditPin(view: PinView | undefined, busy: boolean): boolean {
  return !!view && !view.locked && !busy
}
