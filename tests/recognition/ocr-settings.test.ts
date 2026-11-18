import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { defaultSettings } from '../../src/features/settings/settings'
import type { SystemOcrStatus } from '../../src/features/recognition/ocr'
import type { RecognitionService } from '../../src/features/recognition/services'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), systemOcrStatus: vi.fn(), wechatOcrStatus: vi.fn(), recognitionServiceStatus: vi.fn(), resourceStatus: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import OcrSettings from '../../src/features/recognition/components/OcrSettings.vue'
import ProviderMenu from '../../src/features/recognition/components/ProviderMenu.vue'
clientComponent(ProviderMenu, new URL('../../src/features/recognition/components/ProviderMenu.vue', import.meta.url))
const Component = clientComponent(OcrSettings, new URL('../../src/features/recognition/components/OcrSettings.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const available: SystemOcrStatus = { supported: true, error: null, languages: [{ tag: 'zh-Hans-CN', name: '简体中文' }, { tag: 'en-US', name: 'English' }] }
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const engine = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)!
const select = (label: string) => findNode(root, node => node.props['aria-label'] === label)!
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)
const ocrSpace: RecognitionService = { id: 'saved-ocr', name: 'OCR.space', protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true, keys: ['default'] }
async function mount(props: Record<string, unknown> = {}) { memoryRenderer.render(h(Component, props), root); await flushComponent(); await flushComponent() }
beforeEach(() => {
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  vi.resetAllMocks(); root = new MemoryNode('root'); settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.systemOcrStatus.mockResolvedValue(available)
  api.wechatOcrStatus.mockResolvedValue({ supported: true, detected: true, error: null })
  api.recognitionServiceStatus.mockResolvedValue(false)
  api.resourceStatus.mockResolvedValue({ ocrReady: true })
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('保留默认 PaddleOCR，切换系统引擎仅保存 OCR 配置，页面只显示状态而无资源管理按钮', async () => {
  await mount()
  expect(engine('local-ppocr').props['aria-current']).toBe('page')
  expect(text(engine('local-ppocr'))).toContain('离线')
  expect(text()).toContain('已就绪')
  expect(button('刷新语言包')).toBeUndefined()
  settings.ai.timeoutSeconds = 60; settings.shortcuts.capture = 'F8'
  engine('windows-media').props.onClick(); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'auto', serviceId: '' })
  expect(settings.ai.timeoutSeconds).toBe(60); expect(settings.shortcuts.capture).toBe('F8')
  expect(text()).toContain('已安装 2 个 OCR 语言包')
  expect(select('识别语言').props.options.map((option: { value: string }) => option.value)).toEqual(['auto', 'zh-Hans-CN', 'en-US'])
  select('识别语言').props['onUpdate:modelValue']('en-US'); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'en-US', serviceId: '' })
})

it('PaddleOCR 资源未下载时提示到服务管理下载', async () => {
  api.resourceStatus.mockResolvedValue({ ocrReady: false })
  await mount()
  expect(text()).toContain('未下载识别资源，请在服务管理中下载。')
})

it('系统错误直接显示，保持用户选择且不写入配置', async () => {
  settings.ocr = { provider: 'windows-media', language: 'zh-Hans-CN' }
  api.systemOcrStatus.mockResolvedValueOnce({ supported: true, languages: [], error: 'Windows OCR 调用失败' })
  await mount()
  expect(text()).toContain('Windows OCR 调用失败')
  expect(engine('windows-media').props['aria-current']).toBe('page')
  expect(select('识别语言').props.disabled).toBe(true)
  expect(api.saveSettings).not.toHaveBeenCalled()
})

it('非 Windows 平台禁用系统引擎，但保留已有设置以便主动切换', async () => {
  settings.ocr.provider = 'windows-media'
  api.systemOcrStatus.mockResolvedValueOnce({ supported: false, languages: [], error: null })
  await mount()
  expect(engine('windows-media').props.disabled).toBe(true)
  expect(text()).toContain('Windows 系统 OCR 仅支持 Windows。')
  expect(api.saveSettings).not.toHaveBeenCalled()
  engine('local-ppocr').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
})

it('保存失败保留草稿并支持重试，不改写已生效的引擎', async () => {
  await mount()
  api.saveSettings.mockRejectedValueOnce(new Error('无法保存配置'))
  engine('windows-media').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
  expect(text(findNode(root, node => node.props['data-current-ocr'] !== undefined)!)).toContain('PaddleOCR · PP-OCRv6')
  expect(text(engine('windows-media'))).not.toContain('当前使用')
  expect(engine('windows-media').props['aria-current']).toBe('page')
  expect(text()).toContain('无法保存配置')
  await button('重试')!.props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('windows-media')
  expect(text(engine('windows-media'))).toContain('当前使用')
})

it('未添加在线服务时只列出本地引擎，底部入口跳转服务管理', async () => {
  const manage = vi.fn()
  await mount({ onManage: manage })
  expect(select('OCR 引擎').children.filter(node => node.type === 'button' && node.props['data-provider']).map(node => node.props['data-provider'])).toEqual(['local-ppocr', 'windows-media', 'wechat-ocr'])
  findNode(root, node => node.type === 'button' && node.props['data-provider-manage'] !== undefined)!.props.onClick()
  expect(manage).toHaveBeenCalledOnce()
  expect(api.saveSettings).not.toHaveBeenCalled()
})

it('只列出支持图片的服务，点击直接切换，不出现编辑、测试或密钥表单', async () => {
  settings.recognitionServices = [
    { id: 'text', name: 'Text', protocol: 'openai-compatible', endpoint: 'https://example.com/v1', model: 'text-model', vision: false, keys: ['default'] },
    { id: 'vision', name: 'Vision', protocol: 'openai-compatible', endpoint: 'https://example.com/v1', model: 'vision-model', vision: true, keys: ['default'] },
    ocrSpace,
  ]
  api.recognitionServiceStatus.mockResolvedValue(true)
  await mount()
  expect(engine('service:text')).toBeUndefined()
  expect(text(engine('service:vision'))).toContain('在线 API')
  engine('service:saved-ocr').props.onClick(); await flushComponent(); await flushComponent()
  expect(settings.ocr).toMatchObject({ provider: 'service', serviceId: 'saved-ocr' })
  expect(settings.ai.provider).toBe('local-ctranslate2')
  expect(text()).toContain('已配置密钥')
  expect(findNode(root, node => node.type === 'form' || node.type === 'input')).toBeUndefined()
  for (const label of ['编辑', '测试', '在服务管理中编辑']) expect(button(label)).toBeUndefined()
})

it('缺少密钥的服务也直接切换，并提示到服务管理填写', async () => {
  settings.recognitionServices = [ocrSpace]
  await mount()
  engine('service:saved-ocr').props.onClick(); await flushComponent(); await flushComponent()
  expect(settings.ocr).toMatchObject({ provider: 'service', serviceId: 'saved-ocr' })
  expect(text()).toContain('未配置密钥，请在服务管理中填写。')
})

it('WeChat OCR 作为可选离线引擎保存，只显示检测状态', async () => {
  await mount()
  expect(text(engine('wechat-ocr'))).toContain('WeChat OCR（实验性）')
  engine('wechat-ocr').props.onClick(); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'wechat-ocr', language: 'auto', serviceId: '' })
  expect(text()).toContain('已找到 WeChat OCR 组件')
  expect(button('重新检测')).toBeUndefined()
  expect(findNode(root, node => node.props['aria-label'] === '识别语言')).toBeUndefined()
})

it('不支持的平台禁用 WeChat OCR，保留原选择并允许手动切回默认引擎', async () => {
  settings.ocr.provider = 'wechat-ocr'
  api.wechatOcrStatus.mockResolvedValueOnce({ supported: false, detected: false, error: 'WeChat OCR 仅支持 Windows x64。' })
  await mount()
  expect(engine('wechat-ocr').props.disabled).toBe(true)
  expect(text()).toContain('WeChat OCR 仅支持 Windows x64。')
  expect(api.saveSettings).not.toHaveBeenCalled()
  engine('local-ppocr').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
})

it('识别后动作开关保留，切换引擎保留已保存的开关', async () => {
  settings.ocr = { ...settings.ocr, autoCopy: true, mergeLines: true }
  await mount()
  expect(findNode(root, node => node.props.id === 'ocr-auto-copy')).toBeDefined()
  expect(findNode(root, node => node.props.id === 'ocr-merge-lines')).toBeDefined()
  engine('windows-media').props.onClick(); await flushComponent()
  expect(settings.ocr).toMatchObject({ provider: 'windows-media', autoCopy: true, mergeLines: true })
})
