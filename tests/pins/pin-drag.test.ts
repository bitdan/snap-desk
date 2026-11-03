import { expect, it, vi } from 'vitest'
import { createPinDrag } from '../../src/features/pins/pin-drag'

function host() {
  let captured = false
  const target = { setPointerCapture: vi.fn(() => { captured = true }), hasPointerCapture: () => captured, releasePointerCapture: vi.fn(() => { captured = false }) }
  const moveWindow = vi.fn(), exportContent = vi.fn()
  const drag = createPinDrag(moveWindow, exportContent)
  const event = (patch = {}) => ({ button: 0, buttons: 1, pointerId: 1, clientX: 10, clientY: 10, ctrlKey: false, currentTarget: target, preventDefault: vi.fn(), stopPropagation: vi.fn(), ...patch }) as unknown as PointerEvent
  return { drag, moveWindow, exportContent, target, event }
}

it('普通左键仍移动窗口，右键不触发移动或内容拖出', () => {
  const { drag, event, moveWindow, exportContent } = host()
  drag.down(event({ button: 2, ctrlKey: true }))
  expect(moveWindow).not.toHaveBeenCalled()
  drag.down(event())
  expect(moveWindow).toHaveBeenCalledOnce()
  expect(exportContent).not.toHaveBeenCalled()
})
it('Ctrl 按下启动，超过阈值后松开 Ctrl 仍可拖出，只开始一次', () => {
  const { drag, event, moveWindow, exportContent, target } = host()
  drag.down(event({ ctrlKey: true }))
  drag.move(event({ clientX: 12 }))
  expect(exportContent).not.toHaveBeenCalled()
  drag.move(event({ clientX: 20, ctrlKey: false }))
  expect(target.releasePointerCapture).toHaveBeenCalledExactlyOnceWith(1)
  expect(exportContent).toHaveBeenCalledOnce()
  drag.move(event({ clientX: 100, ctrlKey: false }))
  expect(exportContent).toHaveBeenCalledOnce()
  expect(moveWindow).not.toHaveBeenCalled()
})
it.each(['cancel', 'release', 'otherPointer'])('取消、松开或其他指针不会误启动：%s', mode => {
  const { drag, event, exportContent } = host()
  drag.down(event({ ctrlKey: true }))
  if (mode === 'cancel') drag.cancel()
  drag.move(event({ clientX: 100, buttons: mode === 'release' ? 0 : 1, pointerId: mode === 'otherPointer' ? 2 : 1 }))
  expect(exportContent).not.toHaveBeenCalled()
  drag.cancel()
})
