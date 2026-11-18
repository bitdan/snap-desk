vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { webcrypto } from 'node:crypto'
import { defaultSettings } from '../../src/features/settings/settings'
import { connectionMode, paddleJobsEndpoint, paddleModels, servicePresets, supportsOcr, supportsTranslation, validateService, type RecognitionService } from '../../src/features/recognition/services'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), recognitionServiceStatus: vi.fn(), recognitionServiceCredential: vi.fn(), recognitionServiceKeys: vi.fn(), recognitionServiceTest: vi.fn(), openRecognitionPortal: vi.fn(), resourceStatus: vi.fn(), systemOcrStatus: vi.fn(), wechatOcrStatus: vi.fn(), translationModelStatus: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/resources/components/ResourceSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('resource-settings') }) }
})
vi.mock('../../src/features/recognition/components/LocalTranslationModels.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('local-translation-models') }) }
})
import ServiceSettings from '../../src/features/recognition/components/ServiceSettings.vue'
import ConfirmDialog from '../../src/shared/ui/ConfirmDialog.vue'
clientComponent(ConfirmDialog, new URL('../../src/shared/ui/ConfirmDialog.vue', import.meta.url))
const Component = clientComponent(ServiceSettings, new URL('../../src/features/recognition/components/ServiceSettings.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)!
const input = (label: string) => findNode(findNode(root, node => node.type === 'label' && text(node).startsWith(label))!, node => node.type === 'input')!
const newKey = (index = 0) => findNode(root, node => node.type === 'input' && node.props['aria-label'] === '新密钥' && index-- === 0)!
const row = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)
const service = (id = 'groq-text'): RecognitionService => ({ id, name: 'Groq', protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', vision: false, keys: ['default'] })
const ocrSpace = (id: string, name = 'OCR.space'): RecognitionService => ({ id, name, protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true, keys: ['default'] })
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent() }
async function save() { await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} }); await flushComponent(); await flushComponent() }
async function pick(id: string) {
  findNode(root, node => node.type === 'button' && node.props['data-add-service'] !== undefined)!.props.onClick(); await flushComponent()
  findNode(root, node => node.type === 'button' && node.props['data-preset'] === id)!.props.onClick(); await flushComponent()
}
async function manage(value: string) { row(value)!.props.onClick(); await flushComponent(); await flushComponent() }
beforeEach(() => {
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}); vi.stubGlobal('crypto', webcrypto)
  vi.resetAllMocks(); root = new MemoryNode('root'); settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.recognitionServiceStatus.mockResolvedValue(false)
  api.recognitionServiceCredential.mockResolvedValue(undefined)
  api.recognitionServiceKeys.mockResolvedValue([])
  api.recognitionServiceTest.mockResolvedValue(undefined)
  api.resourceStatus.mockResolvedValue({ ocrReady: true })
  api.systemOcrStatus.mockResolvedValue({ supported: true, languages: [{ tag: 'zh-Hans-CN', name: '简体中文' }], error: null })
  api.wechatOcrStatus.mockResolvedValue({ supported: true, detected: false, error: null })
  api.translationModelStatus.mockResolvedValue({ runtimeReady: true, runtimeSupported: true, models: [] })
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('在线服务只列出已添加的连接；从添加面板选择 DeepSeek，密钥只经凭据命令写入', async () => {
  await mount()
  expect(row('preset:deepseek')).toBeUndefined()
  expect(text()).toContain('尚未添加在线服务')
  await pick('deepseek')
  expect(input('完整接口地址').value).toBe('https://api.deepseek.com/chat/completions')
  expect(input('模型名称').value).toBe('deepseek-flash')
  button('获取 API Key / Token').props.onClick(); await flushComponent()
  expect(api.openRecognitionPortal).toHaveBeenCalledWith('deepseek')
  newKey().props['onUpdate:modelValue']('deepseek-test-key')
  await save()
  const saved = settings.recognitionServices[0]!
  expect(saved).toMatchObject({ name: 'DeepSeek', protocol: 'openai-compatible', model: 'deepseek-flash', vision: true })
  expect(saved.keys).toHaveLength(1); expect(saved.keys[0]).toMatch(/^k-/)
  expect(supportsOcr(saved)).toBe(true); expect(supportsTranslation(saved)).toBe(true)
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(saved.id, saved.endpoint, saved.keys[0], 'deepseek-test-key')
  expect(JSON.stringify(settings)).not.toContain('deepseek-test-key')
  expect(row(`service:${saved.id}`)).toBeDefined()
  expect(settings.ai.provider).toBe('local-ctranslate2'); expect(settings.ocr.provider).toBe('local-ppocr')
})

it('同一平台可添加多个账号，添加面板标记已添加的平台', async () => {
  settings.recognitionServices = [ocrSpace('personal'), ocrSpace('work', 'Work OCR')]
  await mount()
  expect(row('service:personal')).toBeDefined(); expect(row('service:work')).toBeDefined()
  findNode(root, node => node.type === 'button' && node.props['data-add-service'] !== undefined)!.props.onClick(); await flushComponent()
  const tile = findNode(root, node => node.type === 'button' && node.props['data-preset'] === 'ocr-space')!
  expect(findNode(tile, node => node.props.class === 'preset-added')).toBeDefined()
  tile.props.onClick(); await flushComponent()
  newKey().props['onUpdate:modelValue']('third-key'); await save()
  expect(settings.recognitionServices.map(item => item.protocol)).toEqual(['ocr-space', 'ocr-space', 'ocr-space'])
})

it('一个服务可保存多个密钥，按顺序使用，并可单独删除', async () => {
  settings.recognitionServices = [service()]
  api.recognitionServiceKeys.mockResolvedValue([{ id: 'default', hint: 'gsk_…abcd' }])
  await mount(); await manage('service:groq-text')
  expect(text()).toContain('gsk_…abcd')
  expect(findNode(root, node => node.type === 'input' && node.props['aria-label'] === '新密钥')).toBeUndefined()
  button('添加另一个密钥').props.onClick(); await flushComponent()
  newKey().props['onUpdate:modelValue']('second-key'); await flushComponent()
  expect(text()).toContain('请求先用第一个密钥')
  await save()
  const keys = settings.recognitionServices[0]!.keys
  expect(keys[0]).toBe('default'); expect(keys).toHaveLength(2)
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith('groq-text', service().endpoint, keys[1], 'second-key')
  api.recognitionServiceKeys.mockResolvedValue([{ id: 'default', hint: 'gsk_…abcd' }, { id: keys[1], hint: 'seco…-key' }])
  await manage('service:groq-text')
  const remove = findNode(root, node => node.type === 'button' && node.props['aria-label'] === '删除此密钥')!
  await remove.props.onClick(); await flushComponent(); await flushComponent()
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith('groq-text', service().endpoint, 'default', null)
  expect(settings.recognitionServices[0]!.keys).toEqual([keys[1]])
})

it('修改地址后保存会按旧地址清除原密钥，不把原密钥送往新地址', async () => {
  settings.recognitionServices = [service()]
  api.recognitionServiceKeys.mockResolvedValue([{ id: 'default', hint: 'gsk_…abcd' }])
  await mount(); await manage('service:groq-text')
  const endpoint = input('完整接口地址')
  endpoint.props['onUpdate:modelValue']('https://other.example/v1/chat/completions'); endpoint.props.onInput(); await flushComponent()
  expect(text()).toContain('地址已修改')
  expect(text()).not.toContain('gsk_…abcd')
  await save()
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith('groq-text', service().endpoint, 'default', null)
  expect(settings.recognitionServices[0]).toMatchObject({ endpoint: 'https://other.example/v1/chat/completions', keys: [] })
  expect(api.recognitionServiceCredential).not.toHaveBeenCalledWith('groq-text', 'https://other.example/v1/chat/completions', expect.anything(), expect.anything())
})

it('保存凭据失败保留编辑内容，重试不重复创建连接', async () => {
  api.recognitionServiceCredential.mockRejectedValueOnce(new Error('凭据库不可用'))
  await mount(); await pick('groq')
  newKey().props['onUpdate:modelValue']('retry-key'); await save()
  expect(text()).toContain('凭据库不可用'); expect(settings.recognitionServices).toHaveLength(1)
  await save()
  expect(settings.recognitionServices).toHaveLength(1); expect(settings.recognitionServices[0]!.keys).toHaveLength(1)
  expect(text()).toContain('服务已保存')
})

it('管理窗口内测试已保存的服务，成功和失败都在窗口内提示', async () => {
  settings.recognitionServices = [service()]
  await mount(); await manage('service:groq-text')
  const testButton = () => findNode(root, node => node.type === 'button' && node.props['data-service-test'] !== undefined)!
  expect(text(testButton()).trim()).toBe('测试连接')
  await testButton().props.onClick(); await flushComponent()
  expect(api.recognitionServiceTest).toHaveBeenCalledWith('groq-text')
  expect(text()).toContain('测试成功，服务可用。')
  api.recognitionServiceTest.mockRejectedValueOnce(new Error('服务拒绝认证，请检查密钥与模型权限。'))
  await testButton().props.onClick(); await flushComponent()
  expect(text()).toContain('服务拒绝认证')
  // 新建尚未保存的服务测试按钮禁用，保存后才能测试。
  button('取消').props.onClick(); await flushComponent(); await pick('groq')
  expect(testButton().props.disabled).toBe(true)
  expect(findNode(root, node => node.props['data-key-test'] !== undefined)).toBeUndefined()
})

it('逐个测试密钥，只用该密钥请求并在对应行显示结果', async () => {
  settings.recognitionServices = [{ ...service(), keys: ['default', 'k-second'] }]
  api.recognitionServiceKeys.mockResolvedValue([{ id: 'default', hint: 'sk-1…aaaa' }, { id: 'k-second', hint: 'sk-2…bbbb' }])
  await mount(); await manage('service:groq-text'); await flushComponent()
  const rows = () => { const found: MemoryNode[] = []; const walk = (node: MemoryNode) => { if (node.props['data-key-row'] !== undefined) found.push(node); node.children.forEach(walk) }; walk(root); return found }
  const keyTest = (index: number) => findNode(rows()[index]!, node => node.props['data-key-test'] !== undefined)!
  await keyTest(0).props.onClick(); await flushComponent()
  expect(api.recognitionServiceTest).toHaveBeenLastCalledWith('groq-text', 'default')
  expect(text(rows()[0]!)).toContain('可用')
  api.recognitionServiceTest.mockRejectedValueOnce(new Error('服务拒绝认证，请检查密钥与模型权限。'))
  await keyTest(1).props.onClick(); await flushComponent()
  expect(api.recognitionServiceTest).toHaveBeenLastCalledWith('groq-text', 'k-second')
  expect(text(rows()[1]!)).toContain('不可用')
  expect(text(rows()[0]!)).toContain('可用')
})

it('删除前确认并清理全部密钥，正在用于翻译的服务删除后回退到本地引擎', async () => {
  settings.recognitionServices = [{ ...service(), keys: ['default', 'k-second'] }]; settings.ai = { ...settings.ai, provider: 'online-llm', serviceId: 'groq-text' }
  await mount(); await manage('service:groq-text')
  await button('删除服务').props.onClick(); await flushComponent()
  expect(api.recognitionServiceCredential).not.toHaveBeenCalled()
  expect(text()).toContain('删除后翻译将改用本地 CTranslate2。')
  api.recognitionServiceCredential.mockRejectedValueOnce(new Error('无法删除凭据'))
  await button('确认删除').props.onClick(); await flushComponent()
  expect(settings.recognitionServices).toHaveLength(1); expect(api.saveSettings).not.toHaveBeenCalled()
  expect(text()).toContain('无法删除凭据')
  await button('确认删除').props.onClick(); await flushComponent()
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith('groq-text', service().endpoint, 'k-second', null)
  expect(settings.recognitionServices).toHaveLength(0)
  expect(settings.ai).toMatchObject({ provider: 'local-ctranslate2', serviceId: '' })
  expect(findNode(root, node => node.type === 'form')).toBeUndefined()
})

it('删除正在用于 OCR 的服务后回退到 PaddleOCR', async () => {
  settings.recognitionServices = [ocrSpace('saved-ocr')]
  settings.ocr = { ...settings.ocr, provider: 'service', serviceId: 'saved-ocr' }
  await mount(); await manage('service:saved-ocr')
  await button('删除服务').props.onClick(); await flushComponent()
  expect(text()).toContain('删除后 OCR 将改用 PaddleOCR。')
  await button('确认删除').props.onClick(); await flushComponent()
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith('saved-ocr', 'https://api.ocr.space/parse/image', 'default', null)
  expect(settings.recognitionServices).toHaveLength(0)
  expect(settings.ocr).toMatchObject({ provider: 'local-ppocr', serviceId: '' })
})

it('本地引擎在服务管理内打开管理窗口，不跳转到识别或翻译页', async () => {
  await mount()
  const local = (key: string) => findNode(root, node => node.type === 'button' && node.props['data-local-engine'] === key)!
  local('ppocr').props.onClick(); await flushComponent()
  expect(findNode(root, node => node.type === 'resource-settings')).toBeDefined()
  findNode(root, node => node.props['data-window'] === 'PaddleOCR')!.props.onClose(); await flushComponent()
  local('ct2').props.onClick(); await flushComponent()
  expect(findNode(root, node => node.type === 'local-translation-models')).toBeDefined()
  findNode(root, node => node.props['data-window'] === 'CTranslate2')!.props.onClose(); await flushComponent()
  local('wechat').props.onClick(); await flushComponent()
  api.wechatOcrStatus.mockResolvedValueOnce({ supported: true, detected: true, error: null })
  await button('重新检测').props.onClick(); await flushComponent()
  expect(text()).toContain('已找到 WeChat OCR 组件')
})

it('只把明确支持图片的模型用于 OCR，并拒绝不安全连接、错误协议和无效密钥列表', () => {
  expect(connectionMode('http://127.0.0.1:11434/v1')).toBe('local-api')
  expect(connectionMode('http://[::1]:11434/v1')).toBe('local-api')
  expect(connectionMode('https://127.0.0.1.example.com/v1')).toBe('online')
  expect(connectionMode('')).toBeUndefined()
  expect(supportsOcr(service())).toBe(false); expect(supportsTranslation(service())).toBe(true)
  expect(supportsOcr({ ...service(), vision: true })).toBe(true)
  for (const endpoint of ['ftp://remote.example/v1', 'https://key@example.com/v1', 'https://example.com/?token=x', 'file:///tmp/test']) expect(validateService({ ...service(), endpoint })).not.toBeNull()
  expect(validateService({ ...service(), endpoint: 'http://127.0.0.1:11434/v1/chat/completions' })).toBeNull()
  expect(validateService({ ...service(), endpoint: 'http://intranet.example.com:8089/v1' })).toBeNull()
  expect(validateService({ ...service(), protocol: 'ocr-space', model: '4' })).not.toBeNull()
  expect(validateService({ ...service(), protocol: 'deepl' })).not.toBeNull()
  expect(validateService({ ...service(), keys: ['a', 'a'] })).not.toBeNull()
  expect(validateService({ ...service(), keys: ['bad id'] })).not.toBeNull()
  expect(validateService({ ...service(), keys: Array.from({ length: 17 }, (_, index) => `k-${index}`) })).not.toBeNull()
  expect(servicePresets.find(item => item.id === 'groq-vision')!.vision).toBe(true)
})

it('Paddle 默认选择官方异步地址和模型，保存实际 jobs URL', async () => {
  await mount(); await pick('paddle-ocr')
  const endpoint = findNode(root, node => node.props['aria-label'] === 'Paddle 接口地址')!
  const model = findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!
  expect(endpoint.value).toBe('official')
  expect(endpoint.props.options).toContainEqual({ value: 'custom', label: '同步接口（自定义地址）' })
  expect(model.props.options.map((item: { value: string }) => item.value)).toContain('PaddleOCR-VL-1.6')
  expect(endpoint.props.title).toBe(paddleJobsEndpoint)
  model.props['onUpdate:modelValue']('PaddleOCR-VL-1.6')
  newKey().props['onUpdate:modelValue']('test-paddle-token')
  await save()
  const saved = settings.recognitionServices[0]!
  expect(saved).toMatchObject({ protocol: 'paddle-ocr', endpoint: paddleJobsEndpoint, model: 'PaddleOCR-VL-1.6' })
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(saved.id, paddleJobsEndpoint, saved.keys[0], 'test-paddle-token')
})

it('Paddle 旧同步地址保持自定义，切换地址后可恢复自定义地址', async () => {
  settings.recognitionServices = [{ id: 'legacy', name: 'Paddle', protocol: 'paddle-ocr', endpoint: 'https://old.example/ocr', model: 'PP-OCRv6', vision: true, keys: ['default'] }]
  await mount(); await manage('service:legacy')
  const endpoint = () => findNode(root, node => node.props['aria-label'] === 'Paddle 接口地址')!
  expect(endpoint().value).toBe('custom')
  expect(input('同步接口地址').value).toBe('https://old.example/ocr')
  endpoint().props['onUpdate:modelValue']('official'); await flushComponent()
  findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!.props['onUpdate:modelValue']('PP-StructureV3')
  endpoint().props['onUpdate:modelValue']('custom'); await flushComponent()
  expect(input('同步接口地址').value).toBe('https://old.example/ocr')
  expect(findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!.value).toBe('PP-OCRv6')
  expect(api.saveSettings).not.toHaveBeenCalled()
  for (const model of paddleModels) expect(validateService({ ...settings.recognitionServices[0]!, endpoint: paddleJobsEndpoint, model })).toBeNull()
  expect(validateService({ ...settings.recognitionServices[0]!, model: 'PP-StructureV3' })).not.toBeNull()
})
