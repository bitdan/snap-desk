import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, ref } from 'vue'
import { webcrypto } from 'node:crypto'
import { defaultSettings } from '../../src/features/settings/settings'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ aiTranslationConfig: vi.fn(), translateText: vi.fn(), translateTexts: vi.fn(), cancelAiJob: vi.fn(), readTranslationInput: vi.fn(), copyOcrText: vi.fn(), status: vi.fn(), saveSettings: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import TranslationPanel from '../../src/features/recognition/components/TranslationPanel.vue'
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
const Component = clientComponent(TranslationPanel, new URL('../../src/features/recognition/components/TranslationPanel.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const source = ref('hello')
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && (text(node).trim() === label || node.props['aria-label'] === label))!
const select = (label: string) => findNode(root, node => node.type === 'select-control' && node.props['aria-label'] === label)!
const output = () => findNode(root, node => node.props.id === 'translation-output')!.value
const mount = async (autoTranslate = false) => {
  memoryRenderer.render(h({ setup: () => () => h(Component, { workspace: true, autoTranslate, text: source.value, 'onUpdate:text': (value: string) => { source.value = value } }) }), root)
  await flushComponent()
}
beforeEach(() => {
  vi.resetAllMocks(); vi.stubGlobal('crypto', webcrypto)
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  settings = defaultSettings(); root = new MemoryNode('root'); source.value = 'hello'
  api.aiTranslationConfig.mockImplementation(async () => ({ ...settings.ai }))
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.translateText.mockImplementation(async (requestId, _text, _consent, ai) => ({ requestId, translatedText: '你好', targetLanguage: ai.targetLanguage }))
  api.translateTexts.mockImplementation(async (requestId, request) => ({ requestId, translations: [{ sourceText: request.texts[0], translatedText: 'Hello again' }], targetLanguage: request.targetLanguage }))
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

describe('独立文字翻译工作区', () => {
  it('中文自动译成英文，手动指定方向后修改原文也不会覆盖手动选择', async () => {
    source.value = '请检查服务连接是否正常'; await mount(true)
    expect(select('目标语言').value).toBe('en')
    expect(api.translateTexts).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ texts: [source.value], targetLanguage: 'en' }), true, settings.ai)
    expect(settings.ai.targetLanguage).toBe('zh-CN')
    select('目标语言').props['onUpdate:modelValue']('zh-CN'); await flushComponent()
    source.value = '这是新的中文内容'; await flushComponent()
    expect(select('目标语言').value).toBe('zh-CN')
  })
  it('混合比例接近时显示默认方向提示，可手动切换目标', async () => {
    source.value = '你好 hello'; settings.ai.targetLanguage = 'en'; await mount()
    expect(select('目标语言').value).toBe('en')
    expect(findNode(root, node => node.type === 'label' && node.props.class === 'translation-auto')!.props.title).toBe('中英比例接近，使用默认目标语言。')
    select('目标语言').props['onUpdate:modelValue']('zh-CN'); await flushComponent()
    expect(findNode(root, node => node.type === 'label' && node.props.class === 'translation-auto')!.props.title).toBeUndefined()
  })
  it('默认目标为其他语言时保留该目标，不自动覆盖为中英互译', async () => {
    settings.ai = { ...settings.ai, provider: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate', targetLanguage: 'ja' }
    source.value = 'Hello'; await mount()
    expect(select('目标语言').value).toBe('ja')
  })
  it('普通进入等待输入，快捷键进入自动翻译；本地只展示中英', async () => {
    await mount()
    expect(api.translateText).not.toHaveBeenCalled()
    expect(select('目标语言').props.options.map((item: any) => item.value)).toEqual(['zh-CN', 'en'])
    memoryRenderer.render(null, root); await mount(true)
    expect(api.translateText).toHaveBeenCalledExactlyOnceWith(expect.any(String), 'hello', true, settings.ai)
    expect(output()).toBe('你好')
  })
  it('临时语言通过请求传递，保留持久默认配置；修改方向清除旧译文', async () => {
    await mount(); await button('翻译').props.onClick(); await flushComponent()
    expect(output()).toBe('你好')
    select('目标语言').props['onUpdate:modelValue']('en'); await flushComponent()
    expect(output()).toBe('')
    await button('翻译').props.onClick(); await flushComponent()
    expect(api.translateTexts).toHaveBeenCalledExactlyOnceWith(expect.any(String), { texts: ['hello'], sourceLanguage: null, targetLanguage: 'en' }, true, settings.ai)
    expect(settings.ai.targetLanguage).toBe('zh-CN'); expect(api.saveSettings).not.toHaveBeenCalled()
  })
  it('设为默认只合并语言字段，保留其他页面刚保存的配置', async () => {
    await mount(); select('目标语言').props['onUpdate:modelValue']('en'); await flushComponent()
    settings.clipboard.enabled = true; settings.shortcuts.capture = 'F8'; settings.ai.timeoutSeconds = 60
    await button('设为默认语言').props.onClick(); await flushComponent()
    expect(settings.ai.targetLanguage).toBe('en'); expect(settings.ai.timeoutSeconds).toBe(60)
    expect(settings.clipboard.enabled).toBe(true); expect(settings.shortcuts.capture).toBe('F8')
    expect(text(root)).toContain('默认目标语言已保存。')
  })
  it('剪贴板读取保留用户文字，失败可见且不清空原文', async () => {
    await mount(); api.readTranslationInput.mockResolvedValueOnce('clipboard text')
    await button('读取剪贴板').props.onClick(); await flushComponent()
    expect(source.value).toBe('clipboard text'); expect(api.readTranslationInput).toHaveBeenCalledWith('clipboard')
    api.readTranslationInput.mockRejectedValueOnce(new Error('clipboard unavailable'))
    await button('读取剪贴板').props.onClick(); await flushComponent()
    expect(source.value).toBe('clipboard text'); expect(text(root)).toContain('clipboard unavailable')
  })
  it('在线可指定原文语言，交换后以译文作为新原文', async () => {
    settings.ai = { ...settings.ai, provider: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate' }
    await mount(); select('原文语言').props['onUpdate:modelValue']('en'); await flushComponent()
    await button('翻译').props.onClick(); await flushComponent()
    expect(api.translateTexts.mock.calls[0][1].sourceLanguage).toBe('en')
    findNode(root, node => node.type === 'button' && node.props['aria-label'] === '交换语言和文字')!.props.onClick(); await flushComponent()
    expect(source.value).toBe('Hello again'); expect(select('原文语言').value).toBe('zh-CN')
    expect(select('目标语言').value).toBe('en'); expect(output()).toBe('')
  })
  it('Ctrl+Enter 翻译，输入法确认和相同语言不提交请求', async () => {
    await mount(); const panel = findNode(root, node => node.type === 'section')!
    const key = (isComposing = false) => ({ key: 'Enter', ctrlKey: true, isComposing, preventDefault: vi.fn(), stopPropagation: vi.fn() })
    panel.props.onKeydown(key(true)); await flushComponent(); expect(api.translateText).not.toHaveBeenCalled()
    panel.props.onKeydown(key()); await flushComponent(); expect(api.translateText).toHaveBeenCalledOnce()
    select('原文语言').props['onUpdate:modelValue']('zh-CN'); await flushComponent()
    await button('翻译').props.onClick(); await flushComponent()
    expect(text(root)).toContain('原文语言和目标语言不能相同。'); expect(api.translateText).toHaveBeenCalledOnce()
  })
  it('超过 UTF-8 字节限制时不上传', async () => {
    source.value = '中'.repeat(11000); await mount()
    await button('翻译').props.onClick(); await flushComponent()
    expect(api.translateText).not.toHaveBeenCalled(); expect(text(root)).toContain('翻译文字超过 32 KB')
  })
  it('开启翻译后自动复制时译文直接写入剪贴板，默认不复制', async () => {
    await mount(true)
    expect(api.copyOcrText).not.toHaveBeenCalled()
    memoryRenderer.render(null, root); settings.ai.autoCopy = true; await mount(true); await flushComponent()
    expect(api.copyOcrText).toHaveBeenCalledExactlyOnceWith('你好')
  })
})
