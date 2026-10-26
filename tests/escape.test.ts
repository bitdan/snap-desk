import { describe, expect, it, vi } from 'vitest'
import { handleEscape } from '../src/lib/escape'

function key(patch: Partial<KeyboardEvent> = {}) {
  return { key: 'Escape', defaultPrevented: false, isComposing: false, keyCode: 27, repeat: false, preventDefault: vi.fn(), ...patch } as unknown as KeyboardEvent
}
describe('窗口默认 Esc 退出', () => {
  it.each(['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'])('焦点位于 %s 仍退出', tagName => {
    const event = key({ target: { tagName } as HTMLElement }), exit = vi.fn()
    expect(handleEscape(event, exit)).toBe(true)
    expect(exit).toHaveBeenCalledOnce()
    expect(event.preventDefault).toHaveBeenCalledOnce()
  })
  it.each([{ defaultPrevented: true }, { isComposing: true }, { keyCode: 229 }, { repeat: true }, { key: 'Enter' }])('保留内层处理、输入法和长按保护：%o', patch => {
    const exit = vi.fn()
    expect(handleEscape(key(patch), exit)).toBe(false)
    expect(exit).not.toHaveBeenCalled()
  })
})
