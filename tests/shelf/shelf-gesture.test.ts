import { expect, it, vi } from 'vitest'
import { createShelfGesture } from '../../src/features/shelf/shelf-gesture'

function host() {
  let captured = false
  const target = { setPointerCapture: vi.fn(() => { captured = true }), hasPointerCapture: () => captured, releasePointerCapture: vi.fn(() => { captured = false }) }
  const copy = vi.fn(), drag = vi.fn(() => { expect(captured).toBe(false) })
  const gesture = createShelfGesture(copy, drag)
  const pointer = (patch = {}) => ({ button: 0, buttons: 1, pointerId: 1, clientX: 10, clientY: 10, currentTarget: target, ...patch }) as unknown as PointerEvent
  return { target, copy, drag, gesture, pointer }
}

it('小幅移动后的单击只复制；原生拖出释放捕获，并抑制随后生成的单击', () => {
  const { gesture, copy, drag, pointer } = host()
  gesture.down(pointer()); gesture.move(pointer({ clientX: 13 })); gesture.cancel(); gesture.click({ detail: 1 } as MouseEvent)
  expect(copy).toHaveBeenCalledOnce(); expect(drag).not.toHaveBeenCalled()
  gesture.down(pointer()); gesture.move(pointer({ clientX: 18 })); gesture.move(pointer({ clientX: 30 }))
  gesture.click({ detail: 1 } as MouseEvent)
  expect(drag).toHaveBeenCalledOnce(); expect(copy).toHaveBeenCalledOnce()
  gesture.click({ detail: 0 } as MouseEvent)
  expect(copy).toHaveBeenCalledTimes(2)
})

it.each(['cancel', 'release', 'otherPointer', 'rightButton'])('取消或无效指针不拖出：%s', mode => {
  const { gesture, drag, pointer } = host()
  gesture.down(pointer({ button: mode === 'rightButton' ? 2 : 0 }))
  if (mode === 'cancel') gesture.cancel()
  gesture.move(pointer({ clientX: 50, buttons: mode === 'release' ? 0 : 1, pointerId: mode === 'otherPointer' ? 2 : 1 }))
  expect(drag).not.toHaveBeenCalled()
  gesture.cancel()
})
