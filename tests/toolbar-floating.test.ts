import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, shallowRef } from 'vue'
import { useFloatingToolbar } from '../src/lib/toolbar-layout'

// 无需启动 WebView：保留 Vue 响应式，仅收集生命周期回调以验证资源释放。
const lifecycle = vi.hoisted(() => ({ cleanup: [] as (() => void)[] }))
vi.mock('vue', async importOriginal => ({
  ...await importOriginal<typeof import('vue')>(),
  onUnmounted: (callback: () => void) => lifecycle.cleanup.push(callback),
}))
afterEach(() => {
  lifecycle.cleanup.splice(0).forEach(callback => callback())
  vi.unstubAllGlobals()
})

async function setup(initialElement = false) {
  const windowTarget = new EventTarget()
  vi.stubGlobal('window', windowTarget)
  vi.stubGlobal('innerWidth', 1000)
  vi.stubGlobal('innerHeight', 800)
  let measure = () => {}
  const disconnect = vi.fn()
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: () => void) { measure = callback }
    observe() {}
    disconnect = disconnect
  })
  let width = 300
  const node = { getBoundingClientRect: () => ({ left: 100, top: 200, width, height: 80 }) } as HTMLElement
  const element = shallowRef<HTMLElement | null>(initialElement ? node : null)
  const scope = effectScope()
  const floating = scope.run(() => useFloatingToolbar(element))!
  lifecycle.cleanup.push(() => scope.stop())
  element.value = node
  await nextTick()
  const captured = new Set<number>()
  const handle = Object.assign(new EventTarget(), {
    setPointerCapture: (id: number) => captured.add(id),
    hasPointerCapture: (id: number) => captured.has(id),
    releasePointerCapture: (id: number) => captured.delete(id),
  })
  const pointer = (type: string, x: number, y: number, id = 1) => Object.assign(new Event(type), {
    pointerId: id, button: 0, clientX: x, clientY: y,
  }) as PointerEvent
  const start = pointer('pointerdown', 110, 210)
  Object.defineProperty(start, 'currentTarget', { value: handle })
  floating.start(start)
  return { floating, windowTarget, pointer, captured, element, disconnect, handle,
    grow: () => { width = 600; measure() } }
}

describe('浮动工具栏指针生命周期', () => {
  it('传入已有元素时也监听尺寸变化，失去指针捕获后停止移动', async () => {
    const { floating, windowTarget, pointer, grow, handle } = await setup(true)
    windowTarget.dispatchEvent(pointer('pointermove', 910, 710))
    expect(floating.style.value.left).toBe('700px')
    grow()
    expect(floating.style.value.left).toBe('400px')
    handle.dispatchEvent(new Event('lostpointercapture'))
    windowTarget.dispatchEvent(pointer('pointermove', 0, 0))
    expect(floating.style.value.left).toBe('400px')
  })
  it('按手柄偏移移动，忽略其他指针，并采用抬起时的最终位置', async () => {
    const { floating, windowTarget, pointer, captured } = await setup()
    windowTarget.dispatchEvent(pointer('pointermove', 400, 500, 2))
    expect(floating.style.value.left).toBe('100px')
    windowTarget.dispatchEvent(pointer('pointerup', 400, 500, 2))
    expect(captured.has(1)).toBe(true)
    windowTarget.dispatchEvent(pointer('pointerup', 510, 610))
    expect(floating.style.value).toMatchObject({ left: '500px', top: '600px' })
    expect(captured.size).toBe(0)
    windowTarget.dispatchEvent(pointer('pointermove', 0, 0))
    expect(floating.style.value.left).toBe('500px')
  })
  it('窗口缩小和工具栏变宽时保持可见，恢复后交回默认布局', async () => {
    const { floating, windowTarget, pointer, grow } = await setup()
    windowTarget.dispatchEvent(pointer('pointerup', 910, 710))
    expect(floating.style.value.left).toBe('700px')
    grow()
    expect(floating.style.value.left).toBe('400px')
    vi.stubGlobal('innerWidth', 400)
    vi.stubGlobal('innerHeight', 300)
    windowTarget.dispatchEvent(new Event('resize'))
    expect(floating.style.value).toMatchObject({ left: '0px', top: '220px' })
    floating.reset()
    expect(floating.style.value).toEqual({})
  })
  it.each(['blur', 'pointercancel'])('%s 后停止移动；元素移除时断开尺寸监听', async type => {
    const { floating, windowTarget, pointer, captured, element, disconnect } = await setup()
    windowTarget.dispatchEvent(type === 'blur' ? new Event(type) : pointer(type, 500, 600))
    expect(captured.size).toBe(0)
    windowTarget.dispatchEvent(pointer('pointermove', 500, 600))
    expect(floating.style.value.left).toBe('100px')
    element.value = null
    await nextTick()
    expect(disconnect).toHaveBeenCalledOnce()
  })
})
