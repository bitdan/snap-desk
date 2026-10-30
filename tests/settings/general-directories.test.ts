import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import type { DirectoryView } from '../../src/features/settings/types'

const api = vi.hoisted(() => ({ appDirectories: vi.fn(), chooseCaptureDirectory: vi.fn(), chooseRecordingDirectory: vi.fn(), autostartEnabled: vi.fn(), exportSettings: vi.fn(), importSettings: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/features/clipboard/components/ClipboardSettings.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/resources/components/ResourceSettings.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import GeneralSettings from '../../src/features/settings/components/GeneralSettings.vue'
const Component = clientComponent(GeneralSettings, new URL('../../src/features/settings/components/GeneralSettings.vue', import.meta.url))
let root: MemoryNode
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
function row(kind: 'captures' | 'recordings' | 'config') {
  const title = { captures: '截图保存目录', recordings: '录屏保存目录', config: '应用配置目录' }[kind]
  return findNode(root, node => String(node.props.class).includes('directory-row') && text(node).includes(title))!
}
function button(kind: 'captures' | 'recordings' | 'config', label: string) {
  return findNode(row(kind), node => node.type === 'button' && text(node) === label)
}
let directories: DirectoryView[]
beforeEach(() => {
  vi.resetAllMocks(); root = new MemoryNode('root')
  directories = [{ kind: 'captures', path: 'D:/Pictures', error: null }, { kind: 'recordings', path: 'D:/Videos', error: null }, { kind: 'config', path: 'D:/Config', error: null }]
  api.appDirectories.mockImplementation(async () => structuredClone(directories))
  api.autostartEnabled.mockResolvedValue(false)
})
afterEach(() => { memoryRenderer.render(null, root) })
async function mount(props = {}) { memoryRenderer.render(h(Component, props), root); await flushComponent() }

describe('通用设置保存目录', () => {
  it.each(['captures', 'recordings'] as const)('%s 可选择目录并刷新显示，取消选择保留原值', async kind => {
    const choose = kind === 'captures' ? api.chooseCaptureDirectory : api.chooseRecordingDirectory
    choose.mockImplementationOnce(async () => { directories.find(item => item.kind === kind)!.path = 'E:/Selected'; return { path: 'E:/Selected' } })
    await mount()
    await button(kind, '选择')!.props.onClick(); await flushComponent()
    expect(choose).toHaveBeenCalledWith(false)
    expect(text(row(kind))).toContain('E:/Selected')
    choose.mockResolvedValueOnce(null)
    await button(kind, '选择')!.props.onClick(); await flushComponent()
    expect(api.appDirectories).toHaveBeenCalledTimes(2)
    expect(text(row(kind))).toContain('E:/Selected')
  })
  it.each(['captures', 'recordings'] as const)('%s 恢复默认后刷新目录', async kind => {
    const choose = kind === 'captures' ? api.chooseCaptureDirectory : api.chooseRecordingDirectory
    choose.mockImplementation(async () => { directories.find(item => item.kind === kind)!.path = kind === 'captures' ? null : 'D:/Default'; return { path: null } })
    await mount()
    await button(kind, '恢复默认')!.props.onClick(); await flushComponent()
    expect(choose).toHaveBeenCalledWith(true)
    expect(text(row(kind))).toContain(kind === 'captures' ? '系统默认位置' : 'D:/Default')
  })
  it('选择失败保留旧路径并允许重试', async () => {
    api.chooseCaptureDirectory.mockRejectedValueOnce(new Error('目录无法写入'))
    await mount()
    await button('captures', '选择')!.props.onClick(); await flushComponent()
    expect(text(row('captures'))).toContain('D:/Pictures')
    expect(text(findNode(root, node => node.props.role === 'alert')!)).toContain('目录无法写入')
    expect(button('captures', '选择')!.props.disabled).toBe(false)
  })
  it('未记录截图目录时仍可选择，应用配置目录不提供修改入口', async () => {
    directories[0]!.path = null
    await mount()
    expect(text(row('captures'))).toContain('系统默认位置')
    expect(button('captures', '选择')!.props.disabled).toBe(false)
    expect(button('captures', '打开')!.props.disabled).toBe(true)
    expect(button('config', '选择')).toBeUndefined()
    expect(findNode(root, node => node.type === 'button' && ['导入配置', '导出配置'].includes(text(node)))).toBeUndefined()
  })
})


