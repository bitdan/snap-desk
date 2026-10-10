import type { PinView } from './api'

/** 缩放只改变查看比例，按 10% 步进并消除小数累计误差；导出始终使用原图像素。 */
export function stepZoom(zoom: number, direction: number): number {
  return Math.min(4, Math.max(0.1, Math.round((zoom + Math.sign(direction) * 0.1) * 10) / 10))
}
/** 统一菜单和键盘入口的锁定行为，忙碌期间不叠加原生请求。 */
export function canEditPin(view: PinView | undefined, busy: boolean): boolean {
  return !!view && !view.locked && !busy
}
