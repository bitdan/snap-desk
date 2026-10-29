import { afterEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import type { ResourceStatus } from '../../src/features/resources/types'

const api = vi.hoisted(() => ({ resourceStatus: vi.fn(), downloadResources: vi.fn(), chooseExportFont: vi.fn() }))
const subscribe = vi.hoisted(() => vi.fn())
vi.mock('../../src/services/api', () => ({ api, desktop: true, onResourceProgress: subscribe }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Settings from '../../src/features/resources/components/ResourceSettings.vue'
const Component = clientComponent(Settings, new URL('../../src/features/resources/components/ResourceSettings.vue', import.meta.url))
const root = new MemoryNode('root')
const ready: ResourceStatus = { directory: 'resources', ocrReady: true, modelsReady: true, runtimeReady: true, runtimeDownloadMib: 10, fontReady: false }
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const mount = () => memoryRenderer.render(h(Component, { kind: 'ocr' }), root)
afterEach(() => { memoryRenderer.render(null, root); vi.resetAllMocks() })

describe('OCR 设置状态加载', () => {
  it('进度订阅不阻塞状态，重新进入立即展示已确认快照，并在检查后更新', async () => {
    let listen!: (off: () => void) => void, check!: (status: ResourceStatus) => void
    const off = vi.fn()
    subscribe.mockReturnValueOnce(new Promise(resolve => { listen = resolve }))
    api.resourceStatus.mockReturnValueOnce(new Promise(resolve => { check = resolve }))
    mount(); await flushComponent()
    expect(api.resourceStatus).toHaveBeenCalledOnce()
    expect(text()).toContain('正在检查…')
    expect(text()).not.toContain('已就绪')
    check(ready); await flushComponent()
    expect(text()).toContain('已就绪')
    memoryRenderer.render(null, root)
    listen(off); await flushComponent()
    expect(off).toHaveBeenCalledOnce()

    subscribe.mockResolvedValueOnce(vi.fn())
    api.resourceStatus.mockReturnValueOnce(new Promise(resolve => { check = resolve }))
    mount(); await flushComponent()
    expect(text()).toContain('已就绪')
    check({ ...ready, ocrReady: false, modelsReady: false }); await flushComponent()
    expect(text()).not.toContain('已就绪')
    expect(text()).toContain('模型未就绪')
  })
  it('旧页面的迟到查询不会覆盖新页面的资源状态', async () => {
    let old!: (status: ResourceStatus) => void
    subscribe.mockResolvedValue(vi.fn())
    api.resourceStatus.mockReturnValueOnce(new Promise(resolve => { old = resolve }))
    mount(); await flushComponent(); memoryRenderer.render(null, root)
    api.resourceStatus.mockResolvedValueOnce(ready)
    mount(); await flushComponent()
    old({ ...ready, ocrReady: false }); await flushComponent()
    memoryRenderer.render(null, root)
    api.resourceStatus.mockReturnValueOnce(new Promise(() => {}))
    mount(); await flushComponent()
    expect(text()).toContain('已就绪')
  })
  it('主动检查仍执行资源命令，失败后重新读取状态并展示错误', async () => {
    subscribe.mockResolvedValue(vi.fn())
    api.resourceStatus.mockResolvedValueOnce(ready)
    mount(); await flushComponent()
    api.downloadResources.mockRejectedValueOnce(new Error('校验失败'))
    api.resourceStatus.mockResolvedValueOnce({ ...ready, ocrReady: false, runtimeReady: false })
    const check = findNode(root, node => node.type === 'button' && text(node).includes('检查识别资源'))!
    check.props.onClick(); await flushComponent()
    expect(api.downloadResources).toHaveBeenCalledWith('ocr')
    expect(text()).toContain('校验失败')
    expect(text()).not.toContain('已就绪')
  })
})
