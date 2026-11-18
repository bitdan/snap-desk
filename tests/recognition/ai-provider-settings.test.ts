import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'
import type { LocalTranslationStatus } from '../../src/features/recognition/ai'
import type { RecognitionService } from '../../src/features/recognition/services'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), translationModelStatus: vi.fn(), aiProviderStatus: vi.fn() }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Settings from '../../src/features/recognition/components/AiProviderSettings.vue'
import ProviderMenu from '../../src/features/recognition/components/ProviderMenu.vue'
clientComponent(ProviderMenu, new URL('../../src/features/recognition/components/ProviderMenu.vue', import.meta.url))
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
const Component = clientComponent(Settings, new URL('../../src/features/recognition/components/AiProviderSettings.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const ready: LocalTranslationStatus = { runtimeReady: true, runtimeSupported: true, models: [{ modelId: 'opus-mt-en-zh-int8', ready: true, downloadable: true, revision: '1', downloadBytes: 100 }] }
const menu = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)!
const order = () => findNode(root, node => node.type === 'nav' && node.props['aria-label'] === '翻译引擎')!.children.filter(node => node.type === 'button' && node.props['data-provider']).map(node => node.props['data-provider'])
const groq: RecognitionService = { id: 'groq', name: 'Groq', protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', vision: false, keys: ['default'] }
let settings: ReturnType<typeof defaultSettings>
async function mount(props: Record<string, unknown> = {}) { memoryRenderer.render(h(Component, props), root); await flushComponent() }
beforeEach(() => {
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async next => { settings = next; return next })
  api.translationModelStatus.mockResolvedValue(ready); api.aiProviderStatus.mockResolvedValue(false)
})
afterEach(() => { memoryRenderer.render(null, root); vi.resetAllMocks(); vi.unstubAllGlobals() })

it('下拉只列出本地引擎和已添加服务，底部入口跳转服务管理', async () => {
  settings.recognitionServices = [groq]
  const manage = vi.fn()
  await mount({ onManage: manage })
  expect(order()).toEqual(['service:groq', 'offline'])
  findNode(root, node => node.type === 'button' && node.props['data-provider-manage'] !== undefined)!.props.onClick()
  expect(manage).toHaveBeenCalledOnce()
})

it('选择服务直接切换并显示密钥状态，页面不出现密钥输入、测试或删除密钥按钮', async () => {
  settings.recognitionServices = [groq, { ...groq, id: 'second', name: 'Second' }]
  settings.ai.targetLanguage = 'ja'
  api.aiProviderStatus.mockResolvedValue(true)
  await mount()
  menu('service:groq').props.onClick(); await flushComponent()
  expect(settings.ai).toMatchObject({ provider: 'online-llm', serviceId: 'groq', endpoint: groq.endpoint, model: groq.model })
  expect(text()).toContain('已配置密钥')
  menu('service:second').props.onClick(); await flushComponent()
  expect(settings.ai.serviceId).toBe('second'); expect(settings.ocr.provider).toBe('local-ppocr')
  expect(findNode(root, node => node.type === 'input' && node.props.type === 'password')).toBeUndefined()
  for (const label of ['测试已保存配置', '删除已保存密钥', '下载引擎']) expect(findNode(root, node => node.type === 'button' && text(node).trim() === label)).toBeUndefined()
})

it('服务缺少密钥时提示到服务管理填写', async () => {
  settings.recognitionServices = [groq]
  settings.ai = { ...settings.ai, provider: 'online-llm', serviceId: 'groq' }
  await mount(); await flushComponent()
  expect(text()).toContain('未配置密钥，请在服务管理中填写。')
})

it('未能迁移的旧版直接配置保留为旧配置入口，可切回但不可在此编辑', async () => {
  settings.recognitionServices = [groq]
  settings.ai = { ...settings.ai, provider: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate', model: '' }
  await mount()
  expect(menu('legacy').props['aria-current']).toBe('page')
  expect(text(menu('legacy'))).toContain('DeepL · 旧配置')
  expect(findNode(root, node => node.type === 'input' && node.props.type === 'password')).toBeUndefined()
  menu('service:groq').props.onClick(); await flushComponent()
  expect(settings.ai.serviceId).toBe('groq')
  menu('legacy').props.onClick(); await flushComponent()
  expect(settings.ai).toMatchObject({ provider: 'deepl', serviceId: '', endpoint: 'https://api-free.deepl.com/v2/translate' })
})

it('切换服务后，旧的初始化凭据响应不能覆盖新状态', async () => {
  settings.recognitionServices = [groq]
  settings.ai = { ...settings.ai, provider: 'online-llm', endpoint: 'https://old.example/v1', model: 'old-model' }
  let finishOld!: (value: boolean) => void
  api.aiProviderStatus.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve })).mockResolvedValue(false)
  await mount()
  menu('service:groq').props.onClick(); await flushComponent()
  expect(text()).toContain('未配置密钥')
  finishOld(true); await flushComponent()
  expect(text()).toContain('未配置密钥')
})

it('当前使用跟随已生效配置，保存失败时不把正在查看的服务标为当前使用', async () => {
  settings.recognitionServices = [groq]
  await mount()
  const current = () => text(findNode(root, node => node.props['data-current-translation'] !== undefined)!)
  expect(current()).toContain('CTranslate2')
  api.saveSettings.mockRejectedValueOnce(new Error('配置写入失败'))
  menu('service:groq').props.onClick(); await flushComponent()
  expect(menu('service:groq').props['aria-current']).toBe('page')
  expect(current()).toContain('CTranslate2')
  expect(text(menu('service:groq'))).not.toContain('当前使用')
  findNode(root, node => node.type === 'button' && text(node) === '重试')!.props.onClick(); await flushComponent()
  expect(current()).toContain('Groq · openai/gpt-oss-120b')
  expect(text(menu('service:groq'))).toContain('当前使用')
})

it('切换引擎不重排菜单', async () => {
  settings.recognitionServices = [groq, { ...groq, id: 'local-api', name: 'Ollama', endpoint: 'http://localhost:11434/v1/chat/completions', model: 'qwen' }]
  await mount()
  const initial = order()
  for (const value of ['service:groq', 'service:local-api', 'offline']) {
    menu(value).props.onClick(); await flushComponent()
    expect(order()).toEqual(initial)
  }
})

it('本地引擎只显示安装状态，未安装时提示到服务管理安装', async () => {
  await mount(); await flushComponent()
  expect(text()).toContain('本地引擎已安装')
  memoryRenderer.render(null, root)
  api.translationModelStatus.mockResolvedValue({ ...ready, runtimeReady: false, models: [] })
  await mount(); await flushComponent()
  expect(text()).toContain('本地引擎或语言包未安装，请在服务管理中安装。')
  expect(findNode(root, node => node.type === 'button' && text(node).trim() === '下载引擎')).toBeUndefined()
})

it('翻译偏好独立保存，本地引擎只允许中英目标语言', async () => {
  settings.ai.targetLanguage = 'en'
  await mount()
  findNode(root, node => node.props['aria-label'] === '目标语言')!.props['onUpdate:modelValue']('zh-CN'); await flushComponent()
  expect(settings.ai.targetLanguage).toBe('zh-CN')
  expect(findNode(root, node => node.props['aria-label'] === '目标语言')!.props.options.map((item: { value: string }) => item.value)).toEqual(['zh-CN', 'en'])
})
