import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'

const api = vi.hoisted(() => ({ exportSettings: vi.fn(), importSettings: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import ConfigTransferSettings from '../../src/features/settings/components/ConfigTransferSettings.vue'
const Component = clientComponent(ConfigTransferSettings, new URL('../../src/features/settings/components/ConfigTransferSettings.vue', import.meta.url))
let root: MemoryNode
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
beforeEach(() => { vi.resetAllMocks(); root = new MemoryNode('root') })
afterEach(() => { memoryRenderer.render(null, root) })
async function mount(props = {}) { memoryRenderer.render(h(Component, props), root); await flushComponent() }

describe('完整配置导入导出', () => {
  const action = (label: string) => findNode(root, node => node.type === 'button' && text(node) === label)!
  it('导入成功显示结果，取消时不显示成功', async () => {
    const settings = defaultSettings()
    settings.shortcuts.capture = 'Ctrl+Shift+A'
    api.importSettings.mockResolvedValueOnce(settings)
    await mount()
    await action('导入配置').props.onClick(); await flushComponent()
    expect(api.importSettings).toHaveBeenCalledOnce()
    expect(findNode(root, node => node.props.role === 'status' && text(node).includes('配置已导入'))).toBeDefined()
    api.importSettings.mockResolvedValueOnce(null)
    await action('导入配置').props.onClick(); await flushComponent()
    expect(api.importSettings).toHaveBeenCalledTimes(2)
    expect(findNode(root, node => node.props.role === 'status')).toBeUndefined()
  })
  it('导出先等待草稿提交，取消导出不显示成功', async () => {
    const prepareTransfer = vi.fn(async () => true)
    api.exportSettings.mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    await mount({ prepareTransfer })
    await action('导出配置').props.onClick(); await flushComponent()
    expect(prepareTransfer).toHaveBeenCalledOnce()
    expect(findNode(root, node => node.props.role === 'status' && text(node).includes('配置已导出'))).toBeUndefined()
    await action('导出配置').props.onClick(); await flushComponent()
    expect(findNode(root, node => node.props.role === 'status' && text(node).includes('配置已导出'))).toBeDefined()
  })
  it('未保存草稿无效时不启动导入，导入错误保留重试入口', async () => {
    await mount({ prepareTransfer: async () => false })
    await action('导入配置').props.onClick(); await flushComponent()
    expect(api.importSettings).not.toHaveBeenCalled()
    memoryRenderer.render(null, root)
    api.importSettings.mockRejectedValueOnce(new Error('快捷键冲突'))
    await mount()
    await action('导入配置').props.onClick(); await flushComponent()
    expect(text(findNode(root, node => node.props.role === 'alert')!)).toContain('快捷键冲突')
    expect(action('导入配置').props.disabled).toBe(false)
  })
})
