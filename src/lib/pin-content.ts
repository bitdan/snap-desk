import type { ColorPinRequest, TextPinRequest, PinSummary } from './api'
import { prepareRaster } from './annotation-raster'

/** 前端及早提示输入错误；Rust 仍独立校验，不能依赖表单的 min/max 属性。 */
export function normalizePinColor(value: string): string {
  const color = value.trim()
  if (!/^#[\da-f]{6}$/i.test(color)) throw new Error('颜色须为 #RRGGBB，例如 #FFF4B8。')
  return color.toUpperCase()
}

export function colorPinRequest(color: string, width: number, height: number): ColorPinRequest {
  if (![width, height].every(n => Number.isInteger(n) && n >= 16 && n <= 2048)) throw new Error('颜色贴图宽高须为 16～2048 的整数。')
  return { color: normalizePinColor(color), width, height }
}

/** 复用文字标注排版，保留 16 px 留白；长文本无法容纳时提示用户调整，不静默截断。 */
export async function textPinRequest(text: string, color: string, background: string, fontSize: number): Promise<TextPinRequest> {
  if (!text.trim() || [...text].length > 500) throw new Error('请输入 1～500 个字符。')
  if (!Number.isInteger(fontSize) || fontSize < 8 || fontSize > 128) throw new Error('字号须为 8～128 的整数。')
  const ink = normalizePinColor(color), paper = normalizePinColor(background)
  const annotation = await prepareRaster({ kind: 'text', start: { x: 16, y: 16 }, end: { x: 17, y: 17 },
    color: ink, width: 2, opacity: 1, text, fontSize, fontFamily: 'sans' }, '', { x: 16, y: 16, width: 608, height: 480 })
  return { background: paper, annotation }
}

export function pinGroups(pins: PinSummary[]): string[] {
  return [...new Set(pins.map(pin => pin.group).filter(Boolean))].sort((a, b) => a.localeCompare(b))
}

/** null 是全部，空字符串是未分组，两者不可混淆为同一个批量操作范围。 */
export function filterPins(pins: PinSummary[], group: string | null): PinSummary[] {
  return group === null ? pins : pins.filter(pin => pin.group === group)
}

export function normalizePinGroup(value: string): string {
  const group = value.trim()
  if ([...group].length > 32 || /[\u0000-\u001f\u007f-\u009f]/u.test(group)) throw new Error('分组名称最多 32 个字符，且不能包含控制字符。')
  return group
}
