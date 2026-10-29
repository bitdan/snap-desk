import { describe, expect, it } from 'vitest'
import { safeRichStyle } from '../../src/features/pins/html-pin'

describe('富文本贴图样式边界', () => {
  it('保留颜色、强调、段落对齐和有界字号', () => {
    expect(safeRichStyle('color', '#2563eb')).toBe(true)
    expect(safeRichStyle('background-color', 'rgb(255, 240, 200)')).toBe(true)
    expect(safeRichStyle('font-weight', 'bold')).toBe(true)
    expect(safeRichStyle('font-size', '24px')).toBe(true)
    expect(safeRichStyle('text-align', 'center')).toBe(true)
    expect(safeRichStyle('font-size', '999px')).toBe(false)
  })
  it('拒绝外部资源、表达式、变量、布局逃逸和事件相关样式', () => {
    for (const value of ['url(https://private/image)', 'expression(alert(1))', 'var(--secret)', '#fff;position:fixed', '\\75rl(http://private)']) {
      expect(safeRichStyle('background-color', value)).toBe(false)
    }
    expect(safeRichStyle('position', 'fixed')).toBe(false)
    expect(safeRichStyle('background-image', 'url(data:image/svg+xml,secret)')).toBe(false)
    expect(safeRichStyle('behavior', 'url(private)')).toBe(false)
  })
})
