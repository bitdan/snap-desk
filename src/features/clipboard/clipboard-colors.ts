/** 与原生分类保持相同范围，只返回可安全用于色块的规范颜色，不修改剪贴板原文。 */
export function clipboardColor(text: string): string | null {
  const value = text.trim().toLowerCase()
  if (/[\u0000-\u001f\u007f]/u.test(value)) return null
  if (/^#(?:[\da-f]{3}|[\da-f]{4}|[\da-f]{6}|[\da-f]{8})$/.test(value)) return value
  const match = /^(rgb|rgba|hsl|hsla)\(([^()]*)\)$/.exec(value)
  if (!match) return null
  const name = match[1], hsl = name.startsWith('hsl'), alpha = name.endsWith('a')
  const parts = match[2].split(',').map(part => part.trim())
  if (parts.length !== (alpha ? 4 : 3)) return null
  const numbers: number[] = []
  for (const [index, part] of parts.entries()) {
    const percent = part.endsWith('%'), raw = percent ? part.slice(0, -1) : part
    if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/.test(raw)) return null
    if (hsl && (index === 0 && percent || (index === 1 || index === 2) && !percent)) return null
    const number = Number(raw), max = percent ? 100 : index === 3 ? 1 : hsl ? 360 : 255
    if (!Number.isFinite(number) || number < 0 || number > max) return null
    // RGB 的百分比统一为 0～255，alpha 百分比统一为 0～1，避免混合单位被 CSS 拒绝。
    numbers.push(percent && !hsl && index < 3 ? number * 255 / 100 : percent && index === 3 ? number / 100 : number)
  }
  const channels = numbers.map((number, index) => `${number}${hsl && (index === 1 || index === 2) ? '%' : ''}`)
  return `${name}(${channels.join(', ')})`
}
