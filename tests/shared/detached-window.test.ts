import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, ref } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const open = vi.hoisted(() => vi.fn())
vi.mock('../../src/shared/window/detached-window', () => ({ openDetachedWindow: open }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import DetachedWindow from '../../src/shared/ui/DetachedWindow.vue'
const Component = clientComponent(DetachedWindow, new URL('../../src/shared/ui/DetachedWindow.vue', import.meta.url))
let root: MemoryNode, target: MemoryNode
const popup = { closed: false, focus: vi.fn(), document: { title: '' } }
const dispose = vi.fn()
beforeEach(() => {
  vi.clearAllMocks()
  vi.stubGlobal('window', {})
  root = new MemoryNode('root'); target = new MemoryNode('popup')
  open.mockReturnValue({ window: popup, target, dispose })
})
afterEach(async () => { memoryRenderer.render(null, root); await flushComponent(); vi.unstubAllGlobals() })

describe('独立窗口容器', () => {
  it('Esc 直接释放窗口，不依赖父组件更新 open，并可再次打开', async () => {
    const opened = ref(true), close = vi.fn()
    memoryRenderer.render(h({ setup: () => () => h(Component, { open: opened.value, title: '预览', onClose: close }, { default: () => h('p', '预览内容') }) }), root)
    await flushComponent()
    open.mock.calls[0][3]()
    await flushComponent()
    expect(close).toHaveBeenCalledOnce()
    expect(dispose).toHaveBeenCalledOnce()
    expect(findNode(target, node => node.type === 'p')).toBeUndefined()
    opened.value = false; await flushComponent()
    expect(dispose).toHaveBeenCalledOnce()
    opened.value = true; await flushComponent()
    expect(open).toHaveBeenCalledTimes(2)
    expect(findNode(target, node => node.type === 'p')?.text).toBe('预览内容')
  })
  it('内容只挂载在新窗口，业务状态变化直接更新，关闭后释放窗口', async () => {
    const opened = ref(true), value = ref('初始内容')
    memoryRenderer.render(h({ setup: () => () => h(Component, { open: opened.value, title: '预览' }, { default: () => h('p', value.value) }) }), root)
    await flushComponent()
    expect(open).toHaveBeenCalledOnce()
    expect(findNode(root, node => node.type === 'p')).toBeUndefined()
    expect(findNode(target, node => node.type === 'p')?.text).toBe('初始内容')
    value.value = '更新内容'; await flushComponent()
    expect(findNode(target, node => node.type === 'p')?.text).toBe('更新内容')
    opened.value = false; await flushComponent()
    expect(findNode(target, node => node.type === 'p')).toBeUndefined()
    expect(dispose).toHaveBeenCalledOnce()
  })
  it('忙碌时 Esc 不提交关闭，系统关闭后仍能重新打开', async () => {
    const busy = ref(true), close = vi.fn()
    memoryRenderer.render(h({ setup: () => () => h(Component, { open: true, title: '确认', busy: busy.value, onClose: close }) }), root)
    await flushComponent()
    const escape = open.mock.calls[0][3]
    escape(); expect(close).not.toHaveBeenCalled()
    busy.value = false; await flushComponent()
    escape(); expect(close).toHaveBeenCalledOnce()
    busy.value = true; await flushComponent()
    open.mock.calls[0][2](); await flushComponent()
    expect(close).toHaveBeenCalledOnce()
    const retry = findNode(root, node => node.type === 'button')!
    expect(retry).toBeDefined()
    retry.props.onClick(); await flushComponent()
    expect(open).toHaveBeenCalledTimes(2)
  })
  it('浏览器拦截后提供重开入口，卸载时清理已创建的窗口', async () => {
    open.mockReturnValueOnce(null)
    memoryRenderer.render(h(Component, { open: true, title: '更新' }, { default: () => h('p', '内容') }), root)
    await flushComponent()
    expect(findNode(target, node => node.type === 'p')).toBeUndefined()
    findNode(root, node => node.type === 'button')!.props.onClick(); await flushComponent()
    expect(findNode(target, node => node.type === 'p')?.text).toBe('内容')
    memoryRenderer.render(null, root); await flushComponent()
    expect(dispose).toHaveBeenCalledOnce()
  })
  it('嵌套确认窗口归属于已有独立窗口', async () => {
    const nested = new MemoryNode('nested')
    open.mockReturnValueOnce({ window: popup, target, dispose })
      .mockReturnValueOnce({ window: { ...popup }, target: nested, dispose: vi.fn() })
    memoryRenderer.render(h(Component, { open: true, title: '识别' }, {
      default: () => h(Component, { open: true, title: '确认' }, { default: () => h('p', '确认内容') }),
    }), root)
    await flushComponent()
    expect(open).toHaveBeenCalledTimes(2)
    expect(open.mock.calls[1][0]).toBe(popup)
    expect(findNode(nested, node => node.type === 'p')?.text).toBe('确认内容')
  })
})
