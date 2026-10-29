import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'
import type { LocalTranslationStatus } from '../../src/features/recognition/ai'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), aiCredentialSet: vi.fn(), aiProviderTest: vi.fn(), translationModelStatus: vi.fn(), aiProviderStatus: vi.fn(), translationModelAction: vi.fn() }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/resources/components/ResourceSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('ocr-settings') }) }
})
import Settings from '../../src/features/recognition/components/AiProviderSettings.vue'
const Component = clientComponent(Settings, new URL('../../src/features/recognition/components/AiProviderSettings.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const ready: LocalTranslationStatus = { runtimeReady: true, runtimeSupported: true, models: [{ modelId: 'opus-mt-en-zh-int8', ready: true, downloadable: true, revision: '1', downloadBytes: 100 }] }
const mount = () => memoryRenderer.render(h(Component), root)
beforeEach(() => { vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}) })
afterEach(() => { memoryRenderer.render(null, root); vi.resetAllMocks(); vi.unstubAllGlobals() })

describe('翻译设置独立加载和快照复用', () => {
  it('选择 DeepL 后使用官方地址并隐藏模型；切换地址清空未保存密钥', async () => {
    let settings = defaultSettings()
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready)
    api.aiProviderStatus.mockResolvedValue(false)
    mount(); await flushComponent()
    findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === '翻译引擎')!.props['onUpdate:modelValue']('deepl')
    await flushComponent()
    expect(settings.ai.provider).toBe('deepl')
    expect(settings.ai.endpoint).toBe('https://api-free.deepl.com/v2/translate')
    expect(text()).not.toContain('模型名称')
    expect(text()).toContain('测试会发送 Hello，可能产生费用。')
    const key = () => findNode(root, node => node.type === 'input' && node.props.type === 'password')!
    key().props['onUpdate:modelValue']('unsaved-key')
    await flushComponent()
    findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === '服务地址')!.props['onUpdate:modelValue']('https://api.deepl.com/v2/translate')
    await flushComponent()
    expect(key().value).toBe('')
    expect(api.aiCredentialSet).not.toHaveBeenCalled()
  })
  it('慢模型校验不阻塞设置和凭证查询，返回本地页时先显示已有状态', async () => {
    let models!: (status: LocalTranslationStatus) => void
    const settings = defaultSettings(); settings.ai.provider = 'online-llm'
    api.status.mockResolvedValueOnce({ settings })
    api.aiProviderStatus.mockResolvedValueOnce(true)
    api.translationModelStatus.mockReturnValueOnce(new Promise(resolve => { models = resolve }))
    mount(); await flushComponent()
    expect(api.translationModelStatus).toHaveBeenCalledOnce()
    expect(api.aiProviderStatus).toHaveBeenCalledOnce()
    expect(findNode(root, node => node.type === 'fieldset')?.props.disabled).toBe(false)
    expect(text()).toContain('已配置')
    models(ready); await flushComponent(); memoryRenderer.render(null, root)

    api.status.mockResolvedValueOnce({ settings: defaultSettings() })
    api.translationModelStatus.mockReturnValueOnce(new Promise(resolve => { models = resolve }))
    mount(); await flushComponent()
    expect(text()).toContain('本地引擎已安装')
    expect(text()).toContain('已校验')
    models({ ...ready, runtimeReady: false, models: [] }); await flushComponent()
    expect(text()).toContain('翻译引擎未安装或校验失败')
    expect(text()).not.toContain('本地引擎已安装')
  })
  it('查询失败时清除旧状态，设置仍可编辑，资源操作返回后更新快照', async () => {
    api.status.mockResolvedValue({ settings: defaultSettings() })
    api.translationModelStatus.mockRejectedValueOnce(new Error('状态查询失败'))
    mount(); await flushComponent()
    expect(text()).toContain('状态查询失败')
    expect(findNode(root, node => node.type === 'fieldset')?.props.disabled).toBe(false)
    expect(text()).not.toContain('本地引擎已安装')
    memoryRenderer.render(null, root)
    api.translationModelStatus.mockResolvedValueOnce(ready)
    mount(); await flushComponent()
    api.translationModelAction.mockResolvedValueOnce({ ...ready, models: [] })
    findNode(root, node => node.type === 'button' && text(node) === '卸载')!.props.onClick(); await flushComponent()
    expect(api.translationModelAction).toHaveBeenCalledWith('opus-mt-en-zh-int8', 'delete')
    expect(text()).not.toContain('已校验')
  })
})
