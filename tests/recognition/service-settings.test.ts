import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { webcrypto } from 'node:crypto'
import { defaultSettings } from '../../src/features/settings/settings'
import { connectionMode, paddleJobsEndpoint, paddleModels, servicePresets, supportsOcr, supportsTranslation, validateService, type RecognitionService } from '../../src/features/recognition/services'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), recognitionServiceStatus: vi.fn(), recognitionServiceCredential: vi.fn(), openRecognitionPortal: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import ServiceSettings from '../../src/features/recognition/components/ServiceSettings.vue'
import ProviderMenu from '../../src/features/recognition/components/ProviderMenu.vue'
clientComponent(ProviderMenu, new URL('../../src/features/recognition/components/ProviderMenu.vue', import.meta.url))
const Component = clientComponent(ServiceSettings, new URL('../../src/features/recognition/components/ServiceSettings.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)!
const input = (label: string) => findNode(findNode(root, node => node.type === 'label' && text(node).startsWith(label))!, node => node.type === 'input')!
const service = (id = 'groq-text'): RecognitionService => ({ id, name: 'Groq', protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', vision: false })
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent() }
async function save() { await findNode(root, node => node.type === 'form')!.props.onSubmit({ preventDefault() {} }); await flushComponent() }
beforeEach(() => {
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}); vi.stubGlobal('crypto', webcrypto)
  vi.resetAllMocks(); root = new MemoryNode('root'); settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.recognitionServiceStatus.mockResolvedValue(false)
  api.recognitionServiceCredential.mockResolvedValue(undefined)
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('保存多个模型连接，密钥只经凭据命令写入，不进入配置', async () => {
  await mount(); button('添加服务').props.onClick(); await flushComponent()
  input('API Key / Token').props['onUpdate:modelValue']('test-only-key')
  await save()
  expect(settings.recognitionServices).toHaveLength(1)
  const first = settings.recognitionServices[0]!
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(first.id, first.endpoint, 'test-only-key')
  expect(JSON.stringify(settings)).not.toContain('test-only-key')
  expect(settings.ai.provider).toBe('local-ctranslate2'); expect(settings.ocr.provider).toBe('local-ppocr')
  button('添加服务').props.onClick(); await flushComponent(); input('模型名称').props['onUpdate:modelValue']('another-model')
  await save()
  expect(settings.recognitionServices).toHaveLength(2)
  expect(settings.recognitionServices[1]!.id).not.toBe(first.id)
  expect(api.recognitionServiceCredential).toHaveBeenCalledTimes(1)
})
it('修改地址清空未保存密钥，留空保存不把原地址凭据送给新地址', async () => {
  settings.recognitionServices = [service()]
  await mount(); button('编辑').props.onClick(); await flushComponent()
  input('API Key / Token').props['onUpdate:modelValue']('unsaved-key')
  const endpoint = input('完整接口地址')
  endpoint.props['onUpdate:modelValue']('https://other.example/v1/chat/completions'); endpoint.props.onInput()
  await save()
  expect(settings.recognitionServices[0]!.endpoint).toBe('https://other.example/v1/chat/completions')
  expect(api.recognitionServiceCredential).not.toHaveBeenCalled()
})
it('保存凭据失败保留编辑内容，重试不重复创建连接', async () => {
  api.recognitionServiceCredential.mockRejectedValueOnce(new Error('凭据库不可用'))
  await mount(); button('添加服务').props.onClick(); await flushComponent()
  input('API Key / Token').props['onUpdate:modelValue']('retry-key'); await save()
  expect(text()).toContain('凭据库不可用'); expect(settings.recognitionServices).toHaveLength(1)
  await save()
  expect(settings.recognitionServices).toHaveLength(1); expect(text()).toContain('服务已保存')
})
it('正在使用的连接不能删除，删除其他连接前先清理系统凭据', async () => {
  settings.recognitionServices = [service()]; settings.ai = { ...settings.ai, provider: 'online-llm', serviceId: 'groq-text' }
  await mount(); expect(button('删除').props.disabled).toBe(true)
  await button('删除').props.onClick(); expect(api.recognitionServiceCredential).not.toHaveBeenCalled()
  memoryRenderer.render(null, root); settings.ai.provider = 'local-ctranslate2'; await mount()
  api.recognitionServiceCredential.mockRejectedValueOnce(new Error('无法删除凭据'))
  await button('删除').props.onClick(); await flushComponent()
  expect(settings.recognitionServices).toHaveLength(1); expect(api.saveSettings).not.toHaveBeenCalled()
  await button('删除').props.onClick(); await flushComponent()
  expect(settings.recognitionServices).toHaveLength(0)
})
it('只把明确支持图片的模型用于 OCR，并拒绝不安全连接和错误协议', () => {
  expect(connectionMode('http://127.0.0.1:11434/v1')).toBe('local-api')
  expect(connectionMode('http://[::1]:11434/v1')).toBe('local-api')
  expect(connectionMode('https://127.0.0.1.example.com/v1')).toBe('online')
  expect(connectionMode('')).toBeUndefined()
  expect(supportsOcr(service())).toBe(false); expect(supportsTranslation(service())).toBe(true)
  expect(supportsOcr({ ...service(), vision: true })).toBe(true)
  for (const endpoint of ['http://remote.example/v1', 'https://key@example.com/v1', 'https://example.com/?token=x', 'file:///tmp/test']) expect(validateService({ ...service(), endpoint })).not.toBeNull()
  expect(validateService({ ...service(), endpoint: 'http://127.0.0.1:11434/v1/chat/completions' })).toBeNull()
  expect(validateService({ ...service(), protocol: 'ocr-space', model: '4' })).not.toBeNull()
  expect(validateService({ ...service(), protocol: 'deepl' })).not.toBeNull()
  expect(servicePresets.find(item => item.id === 'groq-vision')!.vision).toBe(true)
})


it('点击服务预设直接配置，切换保留普通草稿但清空密钥，浏览菜单不保存配置', async () => {
  await mount()
  const item = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)!
  item('preset:ocr-space').props.onClick(); await flushComponent()
  expect(input('服务名称').value).toBe('OCR.space')
  input('服务名称').props['onUpdate:modelValue']('My OCR account')
  input('API Key / Token').props['onUpdate:modelValue']('unsaved-secret')
  item('preset:groq-vision').props.onClick(); await flushComponent()
  expect(input('服务名称').value).toBe('Groq Vision')
  expect(input('API Key / Token').value).toBe('')
  item('preset:ocr-space').props.onClick(); await flushComponent()
  expect(input('服务名称').value).toBe('My OCR account')
  expect(input('API Key / Token').value).toBe('')
  expect(item('preset:ocr-space').props['aria-current']).toBe('page')
  expect(api.saveSettings).not.toHaveBeenCalled()
  expect(findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === '服务预设')).toBeUndefined()
  await save()
  expect(settings.recognitionServices[0]).toMatchObject({ name: 'My OCR account', protocol: 'ocr-space' })
  expect(api.recognitionServiceCredential).not.toHaveBeenCalled()
})

it('点击已保存连接直接编辑对应账号，保存不覆盖其他连接', async () => {
  settings.recognitionServices = [service('first'), { ...service('second'), name: 'Second account' }]
  await mount()
  findNode(root, node => node.type === 'button' && node.props['data-provider'] === 'service:second')!.props.onClick()
  await flushComponent()
  expect(input('服务名称').value).toBe('Second account')
  input('服务名称').props['onUpdate:modelValue']('Updated second')
  await save()
  expect(settings.recognitionServices.find(item => item.id === 'first')!.name).toBe('Groq')
  expect(settings.recognitionServices.find(item => item.id === 'second')!.name).toBe('Updated second')
  findNode(root, node => node.type === 'button' && node.props['data-provider'] === 'service:first')!.props.onClick()
  await flushComponent(); input('服务名称').props['onUpdate:modelValue']('Updated first'); await save()
  expect(settings.recognitionServices.map(item => item.id)).toEqual(['first', 'second'])

})


it('已保存的供应商替代预设入口，多个真实账号仍各自可编辑', async () => {
  settings.recognitionServices = [
    { id: 'personal', name: 'OCR.space', protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true },
    { id: 'work', name: 'Work OCR', protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true },
  ]
  await mount()
  const item = (value: string) => findNode(root, node => node.type === 'button' && node.props['data-provider'] === value)
  expect(item('preset:ocr-space')).toBeUndefined()
  expect(item('service:personal')).toBeDefined(); expect(item('service:work')).toBeDefined()
  expect(item('preset:paddle-ocr')).toBeDefined()
  item('service:work')!.props.onClick(); await flushComponent()
  expect(input('服务名称').value).toBe('Work OCR')
  expect(api.saveSettings).not.toHaveBeenCalled()
  expect(settings.recognitionServices).toHaveLength(2)
})


it('Paddle 默认选择官方异步地址和模型，保存实际 jobs URL', async () => {
  await mount()
  findNode(root, node => node.type === 'button' && node.props['data-provider'] === 'preset:paddle-ocr')!.props.onClick(); await flushComponent()
  const endpoint = findNode(root, node => node.props['aria-label'] === 'Paddle 接口地址')!
  const model = findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!
  expect(endpoint.value).toBe('official')
  expect(endpoint.props.options).toContainEqual({ value: 'custom', label: '同步接口（自定义地址）' })
  expect(model.props.options.map((item: { value: string }) => item.value)).toContain('PaddleOCR-VL-1.6')
  expect(model.props.options.map((item: { value: string }) => item.value)).toContain('PP-StructureV3')
  expect(text()).toContain(paddleJobsEndpoint)
  model.props['onUpdate:modelValue']('PaddleOCR-VL-1.6')
  input('API Key / Token').props['onUpdate:modelValue']('test-paddle-token')
  await save()
  expect(settings.recognitionServices[0]).toMatchObject({ protocol: 'paddle-ocr', endpoint: paddleJobsEndpoint, model: 'PaddleOCR-VL-1.6' })
  expect(api.recognitionServiceCredential).toHaveBeenCalledWith(settings.recognitionServices[0]!.id, paddleJobsEndpoint, 'test-paddle-token')
})

it('Paddle 旧同步地址保持自定义，切换地址清空密钥并可恢复自定义地址', async () => {
  settings.recognitionServices = [{ id: 'legacy', name: 'Paddle', protocol: 'paddle-ocr', endpoint: 'https://old.example/ocr', model: 'PP-OCRv6', vision: true }]
  await mount(); button('编辑').props.onClick(); await flushComponent()
  const endpoint = () => findNode(root, node => node.props['aria-label'] === 'Paddle 接口地址')!
  expect(endpoint().value).toBe('custom')
  expect(input('同步接口地址').value).toBe('https://old.example/ocr')
  input('API Key / Token').props['onUpdate:modelValue']('unsaved-token')
  endpoint().props['onUpdate:modelValue']('official'); await flushComponent()
  expect(input('API Key / Token').value).toBe('')
  findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!.props['onUpdate:modelValue']('PP-StructureV3')
  endpoint().props['onUpdate:modelValue']('custom'); await flushComponent()
  expect(input('同步接口地址').value).toBe('https://old.example/ocr')
  expect(findNode(root, node => node.props['aria-label'] === 'Paddle 模型')!.value).toBe('PP-OCRv6')
  expect(api.saveSettings).not.toHaveBeenCalled()
  for (const model of paddleModels) expect(validateService({ ...settings.recognitionServices[0]!, endpoint: paddleJobsEndpoint, model })).toBeNull()
  expect(validateService({ ...settings.recognitionServices[0]!, model: 'PP-StructureV3' })).not.toBeNull()
})
