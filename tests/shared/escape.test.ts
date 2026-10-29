import { describe, expect, it, vi } from 'vitest'
import { handleEscape, handleWindowEscape, registerEscapeLayer } from '../../src/shared/ui/escape'
import { effectScope, ref } from 'vue'
import { useEscapeLayer } from '../../src/shared/ui/use-escape-layer'

function key(patch: Partial<KeyboardEvent> = {}) {
  return { key: 'Escape', defaultPrevented: false, isComposing: false, keyCode: 27, repeat: false, preventDefault: vi.fn(), ...patch } as unknown as KeyboardEvent
}
describe('窗口默认 Esc 退出', () => {
  it('按打开顺序逐层退出，活动层外的焦点也不直接退出窗口', () => {
    const exit = vi.fn(), first = vi.fn(), second = vi.fn()
    const offFirst = registerEscapeLayer(() => true, first)
    const offSecond = registerEscapeLayer(() => true, second)
    try {
      expect(handleWindowEscape(key(), exit)).toBe(true)
      expect(second).toHaveBeenCalledOnce()
      expect(first).not.toHaveBeenCalled()
      offSecond()
      handleWindowEscape(key(), exit)
      expect(first).toHaveBeenCalledOnce()
      offFirst()
      handleWindowEscape(key(), exit)
      expect(exit).toHaveBeenCalledOnce()
    } finally { offFirst(); offSecond() }
  })
  it('内层处理过的 Esc、输入法和长按不会继续关闭外层', () => {
    const close = vi.fn(), exit = vi.fn(), off = registerEscapeLayer(() => true, close)
    try {
      for (const patch of [{ defaultPrevented: true }, { isComposing: true }, { repeat: true }]) {
        expect(handleWindowEscape(key(patch), exit)).toBe(false)
      }
      expect(close).not.toHaveBeenCalled(); expect(exit).not.toHaveBeenCalled()
    } finally { off() }
  })
  it('关闭、重新打开和组件卸载同步更新弹层顺序，不残留退出拦截', () => {
    const scope = effectScope(), firstOpen = ref(false), secondOpen = ref(false)
    const first = vi.fn(), second = vi.fn(), exit = vi.fn()
    scope.run(() => {
      useEscapeLayer(() => firstOpen.value, () => { first(); firstOpen.value = false })
      useEscapeLayer(() => secondOpen.value, () => { second(); secondOpen.value = false })
    })
    try {
      firstOpen.value = true; secondOpen.value = true
      firstOpen.value = false; firstOpen.value = true
      handleWindowEscape(key(), exit)
      expect(first).toHaveBeenCalledOnce(); expect(second).not.toHaveBeenCalled()
      handleWindowEscape(key(), exit)
      expect(second).toHaveBeenCalledOnce()
      firstOpen.value = true
      scope.stop()
      handleWindowEscape(key(), exit)
      expect(exit).toHaveBeenCalledOnce()
    } finally { scope.stop() }
  })
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
