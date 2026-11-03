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
vi.mock('../../src/features/recognition/components/OcrSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('ocr-settings') }) }
})
import Settings from '../../src/features/recognition/components/AiProviderSettings.vue'
import ProviderMenu from '../../src/features/recognition/components/ProviderMenu.vue'
clientComponent(ProviderMenu, new URL('../../src/features/recognition/components/ProviderMenu.vue', import.meta.url))
const Component = clientComponent(Settings, new URL('../../src/features/recognition/components/AiProviderSettings.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const ready: LocalTranslationStatus = { runtimeReady: true, runtimeSupported: true, models: [{ modelId: 'opus-mt-en-zh-int8', ready: true, downloadable: true, revision: '1', downloadBytes: 100 }] }
const menu = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)!
const mount = () => memoryRenderer.render(h(Component), root)
beforeEach(() => { vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}) })
afterEach(() => { memoryRenderer.render(null, root); vi.resetAllMocks(); vi.unstubAllGlobals() })

describe('翻译设置独立加载和快照复用', () => {
  it('Groq 预设自动填写地址和模型，隔离原服务密钥，重新进入仍显示 Groq', async () => {
    let settings = defaultSettings()
    settings.ai = { ...settings.ai, provider: 'online-llm', endpoint: 'https://old.example/v1/chat/completions', model: 'old-model', targetLanguage: 'ja' }
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready)
    api.aiProviderStatus.mockResolvedValue(false)
    mount(); await flushComponent()
    expect(text(menu('preset:groq'))).toContain('Groq')
    expect(text(menu('preset:groq'))).toContain('在线 API')
    findNode(root, node => node.type === 'input' && node.props.type === 'password')!.props['onUpdate:modelValue']('old-unsaved-key')
    menu('preset:groq').props.onClick(); await flushComponent()
    expect(settings.ai).toMatchObject({ provider: 'online-llm', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', targetLanguage: 'ja' })
    expect(findNode(root, node => node.type === 'input' && node.props.type === 'password')!.value).toBe('')
    expect(api.aiCredentialSet).not.toHaveBeenCalled()
    memoryRenderer.render(null, root); mount(); await flushComponent()
    expect(menu('preset:groq').props['aria-current']).toBe('page')
    const key = findNode(root, node => node.type === 'input' && node.props.type === 'password')!
    key.props['onUpdate:modelValue']('groq-test-key')
    findNode(root, node => node.type === 'fieldset')!.props.onChange(); await flushComponent()
    expect(api.aiCredentialSet).toHaveBeenCalledWith('groq-test-key', 'https://api.groq.com/openai/v1/chat/completions')
    expect(settings.ai).not.toHaveProperty('apiKey')
  })
  it('旧服务保存结束后不清除新服务重新输入的相同密钥', async () => {
    let settings = defaultSettings()
    settings.ai = { ...settings.ai, provider: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate', model: '' }
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready)
    api.aiProviderStatus.mockResolvedValue(true)
    let finishOld!: () => void
    api.aiCredentialSet.mockReturnValueOnce(new Promise<void>(resolve => { finishOld = resolve })).mockResolvedValue(undefined)
    mount(); await flushComponent()
    const key = () => findNode(root, node => node.type === 'input' && node.props.type === 'password')!
    key().props['onUpdate:modelValue']('shared-test-key')
    findNode(root, node => node.type === 'fieldset')!.props.onChange(); await flushComponent()
    expect(api.aiCredentialSet).toHaveBeenCalledWith('shared-test-key', 'https://api-free.deepl.com/v2/translate')
    findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === '服务地址')!.props['onUpdate:modelValue']('https://api.deepl.com/v2/translate')
    key().props['onUpdate:modelValue']('shared-test-key')
    findNode(root, node => node.type === 'fieldset')!.props.onChange()
    finishOld(); await flushComponent(); await flushComponent()
    expect(api.aiCredentialSet).toHaveBeenCalledTimes(2)
    expect(api.aiCredentialSet).toHaveBeenLastCalledWith('shared-test-key', 'https://api.deepl.com/v2/translate')
  })
  it('切换服务并完成新状态查询后，旧初始化响应不能覆盖密钥状态', async () => {
    let settings = defaultSettings()
    settings.ai = { ...settings.ai, provider: 'online-llm', endpoint: 'https://old.example/v1', model: 'old-model' }
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready)
    let finishOld!: (value: boolean) => void
    api.aiProviderStatus.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve })).mockResolvedValue(false)
    mount(); await flushComponent()
    expect(api.aiProviderStatus).toHaveBeenCalledOnce()
    menu('preset:deepl').props.onClick()
    await flushComponent()
    expect(api.aiProviderStatus).toHaveBeenCalledTimes(2)
    expect(text()).toContain('未配置')
    finishOld(true); await flushComponent()
    expect(text()).toContain('未配置')
    expect(findNode(root, node => node.type === 'button' && text(node) === '删除已保存密钥')!.props.disabled).toBe(true)
  })
  it('选择 DeepL 后使用官方地址并隐藏模型；切换地址清空未保存密钥', async () => {
    let settings = defaultSettings()
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready)
    api.aiProviderStatus.mockResolvedValue(false)
    mount(); await flushComponent()
    expect(menu('offline').props['aria-current']).toBe('page')
    const items = findNode(root, node => node.type === 'nav' && node.props['aria-label'] === '翻译引擎')!.children.filter(node => node.type === 'button')
    expect(items[items.length - 1]!.props['data-provider']).toBe('offline')
    expect(items[0]!.props['data-provider']).not.toBe('preset:local')
    expect(text(menu('offline'))).toContain('离线')
    expect(text(menu('preset:local'))).toContain('本机 API')
    menu('preset:deepl').props.onClick(); await flushComponent()
    expect(settings.ai.provider).toBe('deepl')
    expect(settings.ai.endpoint).toBe('https://api-free.deepl.com/v2/translate')
    expect(findNode(root, node => node.type === 'input' && node.props.placeholder === '填写服务支持的模型 ID')).toBeUndefined()
    expect(text()).toContain('测试会发送 Hello，可能产生费用。')
    const key = () => findNode(root, node => node.type === 'input' && node.props.type === 'password')!
    key().props['onUpdate:modelValue']('unsaved-key')
    await flushComponent()
    findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === '服务地址')!.props['onUpdate:modelValue']('https://api.deepl.com/v2/translate')
    await flushComponent()
    expect(key().value).toBe('')
    expect(api.aiCredentialSet).not.toHaveBeenCalled()
  })
  it('旧 DeepL 配置显示为在线服务，切换平台保留协议、语言并清空未保存密钥', async () => {
    let settings = defaultSettings()
    settings.ai = { ...settings.ai, provider: 'deepl', endpoint: 'https://api.deepl.com/v2/translate', model: '', targetLanguage: 'ja' }
    api.status.mockImplementation(async () => ({ settings }))
    api.saveSettings.mockImplementation(async next => { settings = next; return next })
    api.translationModelStatus.mockResolvedValue(ready); api.aiProviderStatus.mockResolvedValue(false)
    mount(); await flushComponent()
    expect(menu('offline').props['aria-current']).toBeUndefined()
    expect(menu('preset:deepl').props['aria-current']).toBe('page')
    expect(findNode(root, node => node.props['aria-label'] === '服务地址')!.value).toBe('https://api.deepl.com/v2/translate')
    expect(api.saveSettings).not.toHaveBeenCalled()
    findNode(root, node => node.type === 'input' && node.props.type === 'password')!.props['onUpdate:modelValue']('unsaved-deepl-key')
    menu('preset:groq').props.onClick(); await flushComponent()
    expect(settings.ai).toMatchObject({ provider: 'online-llm', endpoint: 'https://api.groq.com/openai/v1/chat/completions', targetLanguage: 'ja' })
    expect(text()).toContain('模型名称'); expect(api.aiCredentialSet).not.toHaveBeenCalled()
    menu('preset:deepl').props.onClick(); await flushComponent()
    expect(settings.ai.provider).toBe('deepl'); expect(findNode(root, node => node.type === 'input' && node.props.placeholder === '填写服务支持的模型 ID')).toBeUndefined()
    menu('preset:custom').props.onClick(); await flushComponent()
    expect(settings.ai).toMatchObject({ provider: 'online-llm', endpoint: '', model: '', targetLanguage: 'ja' })
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


it('翻译独立绑定已保存服务，切换同地址账号也重新读取凭据状态', async () => {
  let settings = defaultSettings()
  settings.recognitionServices = ['first', 'second'].map(id => ({ id, name: id, protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', vision: false }))
  settings.ai.provider = 'online-llm'
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async next => { settings = next; return next })
  api.translationModelStatus.mockResolvedValue(ready); api.aiProviderStatus.mockResolvedValue(true)
  mount(); await flushComponent()
  menu('service:first').props.onClick(); await flushComponent()
  expect(settings.ai.serviceId).toBe('first')
  expect(menu('preset:groq')).toBeUndefined()
  expect(menu('service:first')).toBeDefined(); expect(menu('service:second')).toBeDefined()
  expect(findNode(root, node => node.type === 'input' && node.props.type === 'password')).toBeUndefined()
  menu('service:second').props.onClick(); await flushComponent()
  expect(settings.ai.serviceId).toBe('second'); expect(settings.ocr.provider).toBe('local-ppocr')
  expect(api.aiProviderStatus).toHaveBeenCalledTimes(3)
})


it('当前使用跟随已生效配置，保存失败时不把正在查看的服务标为当前使用', async () => {
  let settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async next => { settings = next; return next })
  api.translationModelStatus.mockResolvedValue(ready); api.aiProviderStatus.mockResolvedValue(false)
  mount(); await flushComponent()
  const current = () => text(findNode(root, node => node.props['data-current-translation'] !== undefined)!)
  expect(current()).toContain('CTranslate2')
  expect(text(menu('offline'))).toContain('当前使用')
  api.saveSettings.mockRejectedValueOnce(new Error('配置写入失败'))
  menu('preset:groq').props.onClick(); await flushComponent()
  expect(menu('preset:groq').props['aria-current']).toBe('page')
  expect(current()).toContain('CTranslate2')
  expect(text(menu('offline'))).toContain('当前使用')
  expect(text(menu('preset:groq'))).not.toContain('当前使用')
  findNode(root, node => node.type === 'button' && text(node) === '重试')!.props.onClick(); await flushComponent()
  expect(current()).toContain('Groq · openai/gpt-oss-120b')
  expect(text(menu('preset:groq'))).toContain('当前使用')
  expect(text(menu('offline'))).not.toContain('当前使用')
})


it('切换平台和当前使用状态不重排菜单，自定义地址变化也保持原位置', async () => {
  let settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async next => { settings = next; return next })
  api.translationModelStatus.mockResolvedValue(ready); api.aiProviderStatus.mockResolvedValue(false)
  mount(); await flushComponent()
  const order = () => findNode(root, node => node.type === 'nav' && node.props['aria-label'] === '翻译引擎')!.children.filter(node => node.type === 'button').map(node => node.props['data-provider'])
  const initial = order()
  for (const value of ['preset:groq', 'preset:custom', 'preset:deepl', 'preset:openai', 'preset:local', 'offline']) {
    menu(value).props.onClick(); await flushComponent()
    expect(order()).toEqual(initial)
  }
})
