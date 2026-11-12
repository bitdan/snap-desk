import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultPinAppearance } from '../../src/features/pins/pins'
import type { PinFrame } from '../../src/features/pins/types'

const api = vi.hoisted(() => ({
  currentPin: vi.fn(), setPinView: vi.fn(), pinAction: vi.fn(), setPinAnnotations: vi.fn(),
  transformPin: vi.fn(), dragPin: vi.fn(), dragPinContent: vi.fn(), setPinRegion: vi.fn(),
}))
vi.mock('../../src/services/api', () => ({ api, onPinVisibility: async () => () => {} }))
vi.mock('../../src/shared/image-protocol', () => ({ pinImageUrl: (label: string, revision: number) => `snap-image://${label}/${revision}.png` }))
vi.mock('../../src/features/pins/events', () => ({ onPinAppearance: async () => () => {}, onPinView: async () => () => {} }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/features/recognition/components/PinRecognition.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/annotations/components/AnnotationSurface.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/annotations/components/AnnotationToolbar.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { slots }) => () => h('toolbar', [slots.leading?.(), slots.trailing?.()]) }) }
})
vi.mock('../../src/features/pins/components/PinMenuButton.vue', () => ({ default: { render: () => null } }))
import PinWindow from '../../src/features/pins/components/PinWindow.vue'
const Component = clientComponent(PinWindow, new URL('../../src/features/pins/components/PinWindow.vue', import.meta.url))

let root: MemoryNode
function frame(expanded: boolean): PinFrame {
  return {
    label: 'pin-1', revision: 1, width: 400, height: 300,
    view: { zoom: 1, opacity: 1, locked: false, topmost: false, toolbarExpanded: expanded },
    appearance: defaultPinAppearance(), transparencySupported: false, annotations: [],
  }
}
const keydown = () => vi.mocked(window.addEventListener).mock.calls.filter(call => call[0] === 'keydown').at(-1)![1] as (event: KeyboardEvent) => void
const stageNode = () => findNode(root, node => String(node.props.class).includes('pin-stage'))!
const controlsNode = () => findNode(root, node => String(node.props.class).includes('pin-controls'))!
function targetFor(node: MemoryNode | null) {
  return { closest: (selector: string) => node && selector.includes('.pin-stage') ? node : null }
}
function press(key: string, target: ReturnType<typeof targetFor>, shiftKey = false) {
  const event = {
    key, shiftKey, ctrlKey: false, metaKey: false, altKey: false,
    defaultPrevented: false, preventDefault: vi.fn(), target,
  } as unknown as KeyboardEvent
  keydown()(event)
  return event
}
const pressTab = (target: ReturnType<typeof targetFor>, shiftKey = false) => press('Tab', target, shiftKey)
async function mount(expanded = true) {
  api.currentPin.mockResolvedValue(frame(expanded))
  memoryRenderer.render(h(Component), root)
  await flushComponent()
}

beforeEach(() => {
  root = new MemoryNode('root'); vi.resetAllMocks()
  vi.stubGlobal('innerWidth', 1000); vi.stubGlobal('innerHeight', 800); vi.stubGlobal('devicePixelRatio', 1)
  vi.stubGlobal('window', { innerWidth: 1000, innerHeight: 800, devicePixelRatio: 1, addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('document', { hasFocus: () => true, querySelector: () => null })
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  Object.assign(MemoryNode.prototype, {
    getBoundingClientRect: () => ({ width: 120, height: 36, left: 0, top: 0 }),
    // 工具栏显示后会测量各行宽度；内存节点没有子行时返回空列表。
    querySelectorAll: () => [],
  })
  api.setPinView.mockImplementation(async view => view)
  api.pinAction.mockResolvedValue(false)
  api.dragPin.mockResolvedValue(undefined)
  api.setPinRegion.mockResolvedValue(undefined)
  api.dragPinContent.mockResolvedValue(undefined)
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

describe('贴图窗口键盘路径', () => {
  it('工具栏展开时从画面按 Tab 直接聚焦第一个控件', async () => {
    await mount(true)
    const first = new MemoryNode('button')
    ;(controlsNode() as unknown as { querySelector: () => MemoryNode }).querySelector = vi.fn(() => first)
    const event = pressTab(targetFor(stageNode()))
    await flushComponent()
    expect(event.preventDefault).toHaveBeenCalledOnce()
    expect(first.focus).toHaveBeenCalledOnce()
    expect(api.setPinView).not.toHaveBeenCalled()
  })

  it('工具栏收起时先展开再送入焦点', async () => {
    await mount(false)
    const first = new MemoryNode('button')
    ;(controlsNode() as unknown as { querySelector: () => MemoryNode }).querySelector = vi.fn(() => first)
    pressTab(targetFor(stageNode()))
    await flushComponent()
    expect(api.setPinView).toHaveBeenCalledWith({ ...frame(false).view, toolbarExpanded: true })
    expect(first.focus).toHaveBeenCalledOnce()
  })

  it('窗口首次 Tab 与 Shift+Tab 保留浏览器默认焦点顺序', async () => {
    await mount(true)
    const body = pressTab(targetFor(null))
    const backward = pressTab(targetFor(stageNode()), true)
    await flushComponent()
    expect(body.preventDefault).not.toHaveBeenCalled()
    expect(backward.preventDefault).not.toHaveBeenCalled()
  })

  it('空格开关工具栏，鼠标悬停不再自动唤出', async () => {
    await mount(false)
    const show = press(' ', targetFor(stageNode()))
    await flushComponent()
    expect(show.preventDefault).toHaveBeenCalledOnce()
    expect(api.setPinView).toHaveBeenLastCalledWith({ ...frame(false).view, toolbarExpanded: true })
    press(' ', targetFor(stageNode()))
    await flushComponent()
    expect(api.setPinView).toHaveBeenLastCalledWith({ ...frame(false).view, toolbarExpanded: false })
    expect(stageNode().props.onPointerenter).toBeUndefined()
  })

  it('工具栏显示时 Esc 先收起工具栏，再按一次才关闭贴图', async () => {
    await mount(true)
    press('Escape', targetFor(stageNode()))
    await flushComponent()
    expect(api.setPinView).toHaveBeenLastCalledWith({ ...frame(true).view, toolbarExpanded: false })
    expect(api.pinAction).not.toHaveBeenCalled()
    press('Escape', targetFor(stageNode()))
    await flushComponent()
    expect(api.pinAction).toHaveBeenCalledWith('close')
  })
})
