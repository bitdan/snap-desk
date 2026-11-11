import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), pinClipboard: vi.fn(), pinFile: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Appearance from '../../src/features/pins/components/PinAppearanceSettings.vue'
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
clientComponent(Appearance, new URL('../../src/features/pins/components/PinAppearanceSettings.vue', import.meta.url))
import Manager from '../../src/features/pins/components/PinManager.vue'
const Component = clientComponent(Manager, new URL('../../src/features/pins/components/PinManager.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node) === label)
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  let settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings: structuredClone(settings) }))
  api.saveSettings.mockImplementation(async value => { settings = structuredClone(value); return structuredClone(settings) })
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })
describe('贴图管理与外观', () => {
  it('剪贴板贴图失败后显示原因，随后可以重新创建', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    api.pinClipboard.mockRejectedValueOnce(new Error('剪贴板没有可读取的图片或文字。'))
    button('剪贴板贴图')!.props.onClick(); await flush()
    expect(text(findNode(root, node => node.props.role === 'alert')!)).toContain('剪贴板没有可读取的图片或文字。')
    expect(button('剪贴板贴图')!.props.disabled).toBe(false)
    api.pinClipboard.mockResolvedValueOnce(undefined)
    button('剪贴板贴图')!.props.onClick(); await flush()
    expect(api.pinClipboard).toHaveBeenCalledTimes(2)
    expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
  })
  it('页面只有创建入口与外观设置，不再列出或筛选贴图', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    expect(findNode(root, node => node.type === 'article')).toBeUndefined()
    expect(findNode(root, node => node.props.id === 'pin-corner')).toBeDefined()
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
  it('选择边框颜色后自动保存，预览同步更新', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    findNode(root, node => node.props.id === 'pin-border-color')!.props['onUpdate:modelValue']('#2563eb')
    findNode(root, node => node.type === 'form')!.props.onChange()
    await flush()
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ pinAppearance: expect.objectContaining({ borderColor: '#2563eb' }) }))
    expect(findNode(root, node => String(node.props.class).includes('preview-sheet'))?.props.style).toMatchObject({ borderColor: '#2563eb' })
  })

})
