vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { webcrypto } from 'node:crypto'
import { defaultSettings } from '../../src/features/settings/settings'
import type { SystemOcrStatus } from '../../src/features/recognition/ocr'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), systemOcrStatus: vi.fn(), wechatOcrStatus: vi.fn(), recognitionServiceStatus: vi.fn(), recognitionServiceCredential: vi.fn(), openRecognitionPortal: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/resources/components/ResourceSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('paddle-resources') }) }
})
import OcrSettings from '../../src/features/recognition/components/OcrSettings.vue'
import ServiceSettings from '../../src/features/recognition/components/ServiceSettings.vue'
clientComponent(ServiceSettings, new URL('../../src/features/recognition/components/ServiceSettings.vue', import.meta.url))
import ProviderMenu from '../../src/features/recognition/components/ProviderMenu.vue'
clientComponent(ProviderMenu, new URL('../../src/features/recognition/components/ProviderMenu.vue', import.meta.url))
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
const Component = clientComponent(OcrSettings, new URL('../../src/features/recognition/components/OcrSettings.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const available: SystemOcrStatus = { supported: true, error: null, languages: [{ tag: 'zh-Hans-CN', name: '简体中文' }, { tag: 'en-US', name: 'English' }] }
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const engine = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)!
const select = (label: string) => findNode(root, node => node.props['aria-label'] === label)!
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)!
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent() }
beforeEach(() => {
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}); vi.stubGlobal('crypto', webcrypto)
  vi.resetAllMocks(); root = new MemoryNode('root'); settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.systemOcrStatus.mockResolvedValue(available)
  api.wechatOcrStatus.mockResolvedValue({ supported: true, detected: true, error: null })
  api.recognitionServiceStatus.mockResolvedValue(false)
  api.recognitionServiceCredential.mockResolvedValue(undefined)
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('保留默认 PaddleOCR，切换系统引擎仅保存 OCR 配置并显示实际语言包', async () => {
  await mount()
  expect(engine('local-ppocr').props['aria-current']).toBe('page')
  expect(text(engine('local-ppocr'))).toContain('离线')
  expect(text(engine('windows-media'))).toContain('离线')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeDefined()
  settings.ai.timeoutSeconds = 60; settings.shortcuts.capture = 'F8'
  engine('windows-media').props.onClick(); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'auto', serviceId: '' })
  expect(settings.ai.timeoutSeconds).toBe(60); expect(settings.shortcuts.capture).toBe('F8')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeUndefined()
  expect(select('识别语言').props.options.map((option: any) => option.value)).toEqual(['auto', 'zh-Hans-CN', 'en-US'])
  select('识别语言').props['onUpdate:modelValue']('en-US'); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'en-US', serviceId: '' })
})

it('系统错误与语言包缺失直接显示，保持用户选择且不写入 PaddleOCR', async () => {
  settings.ocr = { provider: 'windows-media', language: 'zh-Hans-CN' }
  api.systemOcrStatus.mockResolvedValueOnce({ supported: true, languages: [], error: 'Windows OCR 调用失败' })
  await mount()
  expect(text(root)).toContain('Windows OCR 调用失败')
  expect(engine('windows-media').props['aria-current']).toBe('page')
  expect(select('识别语言').props.disabled).toBe(true)
  expect(api.saveSettings).not.toHaveBeenCalled()
  api.systemOcrStatus.mockResolvedValueOnce({ supported: true, languages: [], error: null })
  await button('刷新语言包').props.onClick(); await flushComponent()
  expect(text(root)).toContain('未安装 Windows OCR 语言包')
  expect(settings.ocr.provider).toBe('windows-media')
})

it('非 Windows 平台禁用系统引擎，但保留已有设置以便主动切换', async () => {
  settings.ocr.provider = 'windows-media'
  api.systemOcrStatus.mockResolvedValueOnce({ supported: false, languages: [], error: null })
  await mount()
  expect(engine('windows-media').props.disabled).toBe(true)
  expect(text(root)).toContain('Windows 系统 OCR 仅支持 Windows。')
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
  expect(text(engine('local-ppocr'))).toContain('当前使用')
  expect(text(engine('windows-media'))).not.toContain('当前使用')
  expect(engine('windows-media').props['aria-current']).toBe('page')
  expect(text(root)).toContain('无法保存配置')
  await button('重试').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('windows-media')
  expect(text(findNode(root, node => node.props['data-current-ocr'] !== undefined)!)).toContain('Windows OCR')
  expect(text(engine('windows-media'))).toContain('当前使用')
})

it('语言包查询失败后可刷新恢复，不自动更换引擎或语言', async () => {
  settings.ocr = { provider: 'windows-media', language: 'en-US' }
  api.systemOcrStatus.mockRejectedValueOnce(new Error('语言包查询失败'))
  await mount()
  expect(text(root)).toContain('语言包查询失败')
  await button('刷新语言包').props.onClick(); await flushComponent()
  expect(text(root)).toContain('已安装 2 个 OCR 语言包')
  expect(settings.ocr.language).toBe('en-US')
  expect(api.saveSettings).not.toHaveBeenCalled()
})


it('在线引擎只绑定支持图片的连接，不改变翻译配置，也不展示本地资源下载', async () => {
  settings.recognitionServices = [
    { id: 'text', name: 'Text', protocol: 'openai-compatible', endpoint: 'https://example.com/v1', model: 'text-model', vision: false },
    { id: 'vision', name: 'Vision', protocol: 'openai-compatible', endpoint: 'https://example.com/v1', model: 'vision-model', vision: true },
  ]
  await mount()
  expect(engine('service:vision')).toBeDefined()
  expect(text(engine('service:vision'))).toContain('在线 API')
  expect(engine('service:text')).toBeUndefined()
  expect(select('OCR 引擎').type).toBe('nav')
  engine('service:vision').props.onClick(); await flushComponent()
  expect(settings.ocr).toMatchObject({ provider: 'service', serviceId: 'vision' })
  expect(settings.ai.provider).toBe('local-ctranslate2')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeUndefined()
  // 费用提示直接显示在配置弹窗里，不只放在 tooltip 中。
  expect(text(root)).toContain('OCR 会将选区图片发送到此服务，可能产生费用。')
})


it('未配置时也显示在线 OCR，当前页保存密钥后启用服务，不修改翻译设置', async () => {
  await mount()
  expect(select('OCR 引擎').children.filter(node => node.type === 'button').map(node => node.props['data-provider'])).toEqual(['preset:ocr-space', 'preset:paddle-ocr', 'preset:deepseek', 'local-ppocr', 'windows-media', 'wechat-ocr'])
  expect(text(engine('preset:ocr-space'))).toContain('在线 API')
  expect(text(engine('preset:paddle-ocr'))).toContain('在线 API')
  engine('preset:ocr-space').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
  expect(text(engine('local-ppocr'))).toContain('当前使用')
  expect(text(engine('preset:ocr-space'))).not.toContain('当前使用')
  expect(api.saveSettings).not.toHaveBeenCalled()
  expect(text(root)).not.toContain('1 MB/文件')
  expect(text(root)).toContain('OCR 会将选区图片发送到此服务，可能产生费用。')
  expect(findNode(root, node => node.props['aria-label'] === '服务预设')).toBeUndefined()
  const key = findNode(root, node => node.type === 'input' && node.props.type === 'password')!
  key.props['onUpdate:modelValue']('test-ocr-key')
  await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} })
  await flushComponent(); await flushComponent()
  const service = settings.recognitionServices[0]!
  expect(service.protocol).toBe('ocr-space')
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(service.id, service.endpoint, 'test-ocr-key')
  expect(settings.ocr).toMatchObject({ provider: 'service', serviceId: service.id })
  expect(settings.ai.provider).toBe('local-ctranslate2')
  expect(engine(`service:${service.id}`).props['aria-current']).toBe('page')
  expect(JSON.stringify(settings)).not.toContain('test-ocr-key')
})

it('没有密钥或凭据保存失败时不启用在线 OCR，保留表单供重试', async () => {
  await mount(); engine('preset:ocr-space').props.onClick(); await flushComponent()
  const submit = async () => { await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} }); await flushComponent() }
  await submit()
  expect(text(root)).toContain('请先填写 API Key / Token')
  expect(api.saveSettings).not.toHaveBeenCalled()
  findNode(root, node => node.type === 'input' && node.props.type === 'password')!.props['onUpdate:modelValue']('retry-key')
  api.recognitionServiceCredential.mockRejectedValueOnce(new Error('保存凭据失败'))
  await submit()
  expect(settings.ocr.provider).toBe('local-ppocr')
  expect(text(root)).toContain('保存凭据失败')
  await submit(); await flushComponent()
  expect(settings.recognitionServices).toHaveLength(1)
  expect(settings.ocr.provider).toBe('service')
})


it('DeepSeek 在 OCR 页保存密钥后启用，重进页面用已存连接替代预设', async () => {
  await mount()
  engine('preset:deepseek').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
  expect(api.saveSettings).not.toHaveBeenCalled()
  findNode(root, node => node.type === 'input' && node.props.type === 'password')!.props['onUpdate:modelValue']('deepseek-test-key')
  await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} })
  await flushComponent(); await flushComponent()
  const saved = settings.recognitionServices[0]!
  expect(saved).toMatchObject({ name: 'DeepSeek', model: 'deepseek-flash', vision: true })
  expect(settings.ocr).toMatchObject({ provider: 'service', serviceId: saved.id })
  expect(settings.ai.provider).toBe('local-ctranslate2')
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(saved.id, saved.endpoint, 'deepseek-test-key')
  memoryRenderer.render(null, root); await mount()
  expect(engine('preset:deepseek')).toBeUndefined()
  expect(engine(`service:${saved.id}`).props['aria-current']).toBe('page')
})

it('已保存 OCR.space 时替代添加入口，点击直接编辑原连接且保存不新增同名项', async () => {
  settings.recognitionServices = [{ id: 'saved-ocr', name: 'OCR.space', protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true }]
  settings.ocr = { ...settings.ocr, provider: 'service', serviceId: 'saved-ocr' }
  api.recognitionServiceStatus.mockResolvedValue(true)
  await mount()
  expect(engine('preset:ocr-space')).toBeUndefined()
  expect(engine('service:saved-ocr')).toBeDefined()
  const names = select('OCR 引擎').children.filter(node => node.type === 'button').map(node => text(node))
  expect(names.filter(name => name.includes('OCR.space'))).toHaveLength(1)
  engine('service:saved-ocr').props.onClick(); await flushComponent()
  const input = (label: string) => findNode(findNode(root, node => node.type === 'label' && text(node).startsWith(label))!, node => node.type === 'input')!
  expect(input('服务名称').value).toBe('OCR.space')
  expect(input('完整接口地址').value).toBe('https://api.ocr.space/parse/image')
  input('服务名称').props['onUpdate:modelValue']('My OCR')
  await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} })
  await flushComponent(); await flushComponent()
  expect(settings.recognitionServices).toHaveLength(1)
  expect(settings.recognitionServices[0]).toMatchObject({ id: 'saved-ocr', name: 'My OCR' })
  expect(engine('preset:ocr-space')).toBeUndefined()
  expect(api.recognitionServiceCredential).not.toHaveBeenCalled()
})

it('WeChat OCR 作为可选离线引擎保存，组件检测不改变其他配置', async () => {
  await mount()
  expect(text(engine('wechat-ocr'))).toContain('WeChat OCR（实验性）')
  expect(text(engine('wechat-ocr'))).toContain('离线')
  engine('wechat-ocr').props.onClick(); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'wechat-ocr', language: 'auto', serviceId: '' })
  expect(settings.ai.provider).toBe('local-ctranslate2')
  expect(text(root)).toContain('已找到 WeChat OCR 组件')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeUndefined()
  expect(findNode(root, node => node.props['aria-label'] === '识别语言')).toBeUndefined()
})

it('微信组件缺失或查询失败时显示原因，刷新恢复且不自动切换引擎', async () => {
  settings.ocr.provider = 'wechat-ocr'
  api.wechatOcrStatus.mockResolvedValueOnce({ supported: true, detected: false, error: '未找到 WeChat OCR 插件。打开微信，对任意图片右键 →「提取文字」，微信会自动下载插件。完成后点击“重新检测”。' })
  await mount()
  expect(text(root)).toContain('未找到 WeChat OCR 插件。打开微信，对任意图片右键 →「提取文字」，微信会自动下载插件。完成后点击“重新检测”。')
  expect(api.saveSettings).not.toHaveBeenCalled()
  api.wechatOcrStatus.mockRejectedValueOnce(new Error('检测失败'))
  await button('重新检测').props.onClick(); await flushComponent()
  expect(text(root)).toContain('检测失败')
  await button('重新检测').props.onClick(); await flushComponent()
  expect(text(root)).toContain('已找到 WeChat OCR 组件')
  expect(settings.ocr.provider).toBe('wechat-ocr')
  expect(api.saveSettings).not.toHaveBeenCalled()
})

it('不支持的平台禁用 WeChat OCR，保留原选择并允许手动切回默认引擎', async () => {
  settings.ocr.provider = 'wechat-ocr'
  api.wechatOcrStatus.mockResolvedValueOnce({ supported: false, detected: false, error: 'WeChat OCR 仅支持 Windows x64。' })
  await mount()
  expect(engine('wechat-ocr').props.disabled).toBe(true)
  expect(text(root)).toContain('WeChat OCR 仅支持 Windows x64。')
  expect(api.saveSettings).not.toHaveBeenCalled()
  engine('local-ppocr').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
})

it('初次微信检测失败仍可点击，进入面板查看错误并重新检测', async () => {
  api.wechatOcrStatus.mockRejectedValueOnce(new Error('wechat_ocr_status 调用失败'))
  await mount()
  expect(engine('wechat-ocr').props.disabled).toBe(false)
  engine('wechat-ocr').props.onClick(); await flushComponent()
  expect(text(root)).toContain('wechat_ocr_status 调用失败')
  expect(button('重新检测')).toBeDefined()
  await button('重新检测').props.onClick(); await flushComponent()
  expect(text(root)).toContain('已找到 WeChat OCR 组件')
  expect(settings.ocr.provider).toBe('wechat-ocr')
})

it('微信检测尚未完成时可打开面板，显示检测中而非误判为不支持', async () => {
  let resolve!: (status: { supported: boolean; detected: boolean; error: null }) => void
  api.wechatOcrStatus.mockImplementationOnce(() => new Promise(done => { resolve = done }))
  await mount()
  expect(engine('wechat-ocr').props.disabled).toBe(false)
  engine('wechat-ocr').props.onClick(); await flushComponent()
  expect(text(root)).toContain('正在检测 WeChat OCR…')
  expect(button('重新检测').props.disabled).toBe(true)
  resolve({ supported: true, detected: true, error: null }); await flushComponent()
  expect(text(root)).toContain('已找到 WeChat OCR 组件')
})

it('识别后动作开关随引擎面板显示，切换引擎保留已保存的开关', async () => {
  settings.ocr = { ...settings.ocr, autoCopy: true, mergeLines: true }
  await mount()
  expect(findNode(root, node => node.props.id === 'ocr-auto-copy')).toBeDefined()
  expect(findNode(root, node => node.props.id === 'ocr-merge-lines')).toBeDefined()
  engine('windows-media').props.onClick(); await flushComponent()
  expect(settings.ocr).toMatchObject({ provider: 'windows-media', autoCopy: true, mergeLines: true })
})
