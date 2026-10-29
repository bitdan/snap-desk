import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), listPins: vi.fn(), managePins: vi.fn(), pinClipboard: vi.fn(), pinFile: vi.fn(), setPinGroup: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true, onPinsChanged: async () => () => {} }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Appearance from '../../src/features/pins/components/PinAppearanceSettings.vue'
clientComponent(Appearance, new URL('../../src/features/pins/components/PinAppearanceSettings.vue', import.meta.url))
import Manager from '../../src/features/pins/components/PinManager.vue'
const Component = clientComponent(Manager, new URL('../../src/features/pins/components/PinManager.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node) === label)
const entries = () => { const result: MemoryNode[] = []; function visit(node: MemoryNode) { if (node.type === 'article') result.push(node); node.children.forEach(visit) }; visit(root); return result }
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  let settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings: structuredClone(settings) }))
  api.saveSettings.mockImplementation(async value => { settings = structuredClone(value); return structuredClone(settings) })
  api.listPins.mockResolvedValue([
    { label: 'pin-1', width: 640, height: 480, visible: true, group: '', thumbnail: 'data:image/png;base64,one' },
    { label: 'pin-2', width: 320, height: 180, visible: false, group: '参考', thumbnail: 'data:image/png;base64,two' },
  ])
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })
describe('贴图管理与外观', () => {
  it('穿透贴图无需点击图片本身即可从管理页恢复鼠标操作', async () => {
    api.listPins.mockResolvedValue([{ label: 'pin-1', width: 640, height: 480, visible: true, group: '', thumbnail: '', clickThrough: true }])
    memoryRenderer.render(h(Component), root); await flush()
    button('恢复鼠标操作')!.props.onClick(); await flush()
    expect(api.managePins).toHaveBeenCalledExactlyOnceWith('interactive', 'pin-1', null)
  })
  it('显示真实缩略图，按显示状态和分组筛选，未选择时没有批量操作', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    expect(entries()).toHaveLength(2)
    expect(findNode(root, node => node.type === 'img')?.props.src).toBe('data:image/png;base64,one')
    expect(findNode(root, node => node.props['aria-label'] === '批量操作')).toBeUndefined()
    button('已隐藏')!.props.onClick(); await nextTick()
    expect(entries()).toHaveLength(1); expect(text(entries()[0])).toContain('pin-2')
    findNode(root, node => node.props['aria-label'] === '筛选贴图分组')!.props['onUpdate:modelValue']('')
    await nextTick(); expect(entries()).toHaveLength(0)
  })
  it('批量操作只针对选择的贴图，操作后仍可继续管理', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    findNode(root, node => node.props.id === 'pin-choice-pin-1')!.props['onUpdate:modelValue'](['pin-1'])
    await nextTick()
    button('隐藏')!.props.onClick(); await flush()
    expect(api.managePins).toHaveBeenCalledExactlyOnceWith('hide', 'pin-1', null)
    expect(entries()).toHaveLength(2)
  })
  it('圆角与阴影自动保存，并保留其他模块的配置', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    findNode(root, node => node.props.id === 'pin-shadow')!.props['onUpdate:modelValue']('none')
    const radius = findNode(root, node => node.props.id === 'pin-corner')!
    radius.props['onUpdate:modelValue'](16)
    findNode(root, node => node.type === 'form')!.props.onInput()
    await vi.advanceTimersByTimeAsync(350)
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ pinAppearance: { shadow: 'none', cornerRadius: 16, borderColor: '#a1a1aa' }, export: expect.objectContaining({ filenamePrefix: 'SnapDesk' }) }))
    expect(findNode(root, node => String(node.props.class).includes('preview-sheet'))?.props.style).toMatchObject({ borderRadius: '16px', boxShadow: 'none' })
  })
  it('选择边框颜色后自动保存，预览与缩略图同步更新', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    findNode(root, node => node.props.id === 'pin-border-color')!.props['onUpdate:modelValue']('#2563eb')
    findNode(root, node => node.type === 'form')!.props.onChange()
    await flush()
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ pinAppearance: expect.objectContaining({ borderColor: '#2563eb' }) }))
    expect(findNode(root, node => String(node.props.class).includes('preview-sheet'))?.props.style).toMatchObject({ borderColor: '#2563eb' })
    expect(findNode(root, node => node.props.class === 'pin-thumbnail')?.props.style).toMatchObject({ borderColor: '#2563eb' })
  })

})
