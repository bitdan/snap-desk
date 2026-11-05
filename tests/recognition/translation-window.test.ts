import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { webcrypto } from 'node:crypto'
import { defaultAiSettings } from '../../src/features/recognition/ai'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ readTranslationInput: vi.fn(), aiTranslationConfig: vi.fn(), translateText: vi.fn(), translateTexts: vi.fn(), cancelAiJob: vi.fn(), closeTranslation: vi.fn(), controlTranslationWindow: vi.fn(), startTranslationCapture: vi.fn(), copyOcrText: vi.fn() }))
const events = vi.hoisted(() => ({ open: () => {}, off: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, onTranslationOpen: async (fn: () => void) => { events.open = fn; return events.off } }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import TranslationWindow from '../../src/features/recognition/components/TranslationWindow.vue'
import TranslationPanel from '../../src/features/recognition/components/TranslationPanel.vue'
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
clientComponent(TranslationPanel, new URL('../../src/features/recognition/components/TranslationPanel.vue', import.meta.url))
const Component = clientComponent(TranslationWindow, new URL('../../src/features/recognition/components/TranslationWindow.vue', import.meta.url))
let root: MemoryNode
const content = (node: MemoryNode): string => node.text + node.children.map(content).join('')
const source = () => findNode(root, node => node.props.id === 'translation-source')!
const button = (label: string) => findNode(root, node => node.type === 'button' && (content(node).trim() === label || node.props['aria-label'] === label))!
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent() }
beforeEach(() => {
  vi.resetAllMocks(); root = new MemoryNode('root')
  vi.stubGlobal('crypto', webcrypto); vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  api.readTranslationInput.mockResolvedValue(null)
  api.aiTranslationConfig.mockResolvedValue(defaultAiSettings())
  api.cancelAiJob.mockResolvedValue(undefined)
  api.translateText.mockImplementation(async requestId => ({ requestId, translatedText: '你好', targetLanguage: 'zh-CN' }))
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('普通打开保持空白，不读取剪贴板或自动发送请求', async () => {
  await mount()
  expect(source().value).toBe('')
  expect(api.readTranslationInput).toHaveBeenCalledExactlyOnceWith('pending')
  expect(api.translateText).not.toHaveBeenCalled()
  expect(content(root)).not.toContain('设为默认语言')
})
it('冷启动补读原文并自动翻译，重复空通知保留当前内容', async () => {
  api.readTranslationInput.mockResolvedValueOnce('hello')
  await mount()
  expect(source().value).toBe('hello')
  expect(api.translateText).toHaveBeenCalledOnce()
  events.open(); await flushComponent()
  expect(source().value).toBe('hello')
  expect(api.translateText).toHaveBeenCalledOnce()
})
it('新原文替换时取消旧请求，忽略旧结果并翻译新原文', async () => {
  let finish!: (value: unknown) => void
  api.readTranslationInput.mockResolvedValueOnce('first text')
  api.translateText.mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
  await mount()
  const previous = api.translateText.mock.calls[0][0]
  api.readTranslationInput.mockResolvedValueOnce('second text')
  events.open(); await flushComponent(); await flushComponent()
  expect(api.cancelAiJob).toHaveBeenCalledWith(previous)
  expect(source().value).toBe('second text')
  expect(api.translateText).toHaveBeenLastCalledWith(expect.any(String), 'second text', true, defaultAiSettings())
  finish({ requestId: previous, translatedText: '旧译文', targetLanguage: 'zh-CN' }); await flushComponent()
  expect(findNode(root, node => node.props.id === 'translation-output')!.value).toBe('你好')
})
it('截图失败保留原文并允许重试，关闭按钮走独立窗口命令', async () => {
  await mount()
  source().props['onUpdate:modelValue']('draft'); await flushComponent()
  api.startTranslationCapture.mockRejectedValueOnce(new Error('截图失败'))
  await button('截图翻译').props.onClick(); await flushComponent()
  expect(content(root)).toContain('截图失败')
  expect(source().value).toBe('draft')
  await button('截图翻译').props.onClick()
  expect(api.startTranslationCapture).toHaveBeenCalledTimes(2)
  await findNode(root, node => node.props['aria-label'] === '关闭翻译')!.props.onClick()
  expect(api.closeTranslation).toHaveBeenCalledOnce()
  memoryRenderer.render(null, root)
  expect(events.off).toHaveBeenCalledOnce()
})
it('Enter 翻译，Shift+Enter 和输入法确认保留输入，Esc 关闭窗口', async () => {
  await mount()
  source().props['onUpdate:modelValue']('hello'); await flushComponent()
  const panel = findNode(root, node => typeof node.props.onKeydown === 'function' && node.type === 'section')!
  const key = (patch = {}) => ({ key: 'Enter', target: { tagName: 'TEXTAREA', id: 'translation-source' }, preventDefault: vi.fn(), stopPropagation: vi.fn(), ...patch })
  panel.props.onKeydown(key({ shiftKey: true })); panel.props.onKeydown(key({ isComposing: true }))
  expect(api.translateText).not.toHaveBeenCalled()
  panel.props.onKeydown(key()); await flushComponent()
  expect(api.translateText).toHaveBeenCalledOnce()
  panel.props.onKeydown(key({ key: 'Escape' })); await flushComponent()
  expect(api.closeTranslation).toHaveBeenCalledOnce()
})

it('顶部置顶状态只在原生命令成功后改变，拖动只响应左键', async () => {
  await mount()
  api.controlTranslationWindow.mockRejectedValueOnce(new Error('无法置顶'))
  await button('置顶窗口').props.onClick(); await flushComponent()
  expect(button('置顶窗口').props['aria-pressed']).toBe(false)
  await button('置顶窗口').props.onClick(); await flushComponent()
  expect(button('取消置顶').props['aria-pressed']).toBe(true)
  await button('取消置顶').props.onClick(); await flushComponent()
  expect(api.controlTranslationWindow).toHaveBeenLastCalledWith('unpin')
  const drag = findNode(root, node => node.props.class === 'window-drag')!
  drag.props.onPointerdown({ button: 2 })
  expect(api.controlTranslationWindow).toHaveBeenCalledTimes(3)
  await drag.props.onPointerdown({ button: 0 })
  expect(api.controlTranslationWindow).toHaveBeenLastCalledWith('drag')
})

it('原文工具可合并 OCR 换行并复制，结果卡片支持按交换后的语言回译', async () => {
  await mount()
  source().props['onUpdate:modelValue']('trans-\nlation\ntext'); await flushComponent()
  await button('合并换行').props.onClick(); await flushComponent()
  expect(source().value).toBe('translation text')
  await button('复制原文').props.onClick()
  expect(api.copyOcrText).toHaveBeenCalledExactlyOnceWith('translation text')
  await button('翻译').props.onClick(); await flushComponent()
  expect(content(root)).toContain('本地翻译')
  api.translateTexts.mockImplementationOnce(async requestId => ({ requestId, translations: [{ sourceText: '你好', translatedText: 'hello' }], targetLanguage: 'en' }))
  await button('回译').props.onClick(); await flushComponent()
  expect(source().value).toBe('你好')
  expect(api.translateTexts).toHaveBeenCalledExactlyOnceWith(expect.any(String), { texts: ['你好'], sourceLanguage: 'zh-CN', targetLanguage: 'en' }, true, defaultAiSettings())
})
