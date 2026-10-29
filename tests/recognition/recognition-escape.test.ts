import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, type Component } from 'vue'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { webcrypto } from 'node:crypto'
import { handleEscape } from '../../src/shared/ui/escape'
import { defaultAiSettings } from '../../src/features/recognition/ai'
import { clientComponent, MemoryNode, memoryRenderer, findNode, flushComponent } from '../helpers/component-host'
const api = vi.hoisted(() => ({ recognizePinText: vi.fn(), recognizePinQr: vi.fn(), aiTranslationConfig: vi.fn(), translateText: vi.fn(), translateTexts: vi.fn(), cancelAiJob: vi.fn(), copyOcrText: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import PinRecognition from '../../src/features/recognition/components/PinRecognition.vue'
import ImageTranslation from '../../src/features/recognition/components/ImageTranslation.vue'
import TranslationPanel from '../../src/features/recognition/components/TranslationPanel.vue'
import OcrResultCard from '../../src/features/recognition/components/OcrResultCard.vue'
import QrResultCard from '../../src/features/recognition/components/QrResultCard.vue'
const components = { PinRecognition, ImageTranslation, TranslationPanel, OcrResultCard, QrResultCard }
const recognition = { provider: 'local-ppocr', fullText: 'hello', lines: [], imageWidth: 640, imageHeight: 480 }
const frame = { label: 'pin-1', width: 640, height: 480, dataUrl: '', view: { zoom: 1, opacity: 1, locked: false, topmost: true, toolbarExpanded: true }, annotations: [], transparencySupported: true }
const clients: Record<string, Component> = Object.fromEntries(Object.entries(components).map(([name, component]) => [name, clientComponent(component, new URL(`../../src/features/recognition/components/${name}.vue`, import.meta.url))]))
let root = new MemoryNode('root'), close = vi.fn()
async function mount(name: keyof typeof components, overrides = {}) {
  const props = { onClose: close, text: 'hello', frame, mode: 'ocr', recognition, image: { dataUrl: '', width: 640, height: 480 }, captureResult: { id: 7, items: [] }, ...overrides }
  memoryRenderer.render(h(clients[name], props), root)
  await flushComponent()
  return findNode(root, node => typeof node.props.onKeydown === 'function')!
}
function escape(patch: Record<string, unknown> = {}) {
  const event = { key: 'Escape', defaultPrevented: false, isComposing: false, keyCode: 27, repeat: false,
    target: { tagName: 'TEXTAREA' }, stopPropagation: vi.fn(), preventDefault: vi.fn(() => { event.defaultPrevented = true }), ...patch }
  return event as unknown as KeyboardEvent
}
function content(node: MemoryNode): string { return node.text + node.children.map(content).join('') }
beforeEach(() => {
  vi.resetAllMocks(); root = new MemoryNode('root'); close = vi.fn()
  vi.stubGlobal('crypto', webcrypto)
  vi.stubGlobal('Document', class {})
  vi.stubGlobal('ShadowRoot', class {})
  api.recognizePinText.mockReturnValue(new Promise(() => {}))
  api.aiTranslationConfig.mockResolvedValue(defaultAiSettings())
  api.translateText.mockReturnValue(new Promise(() => {}))
  api.cancelAiJob.mockRejectedValue(new Error('already finished'))
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

describe('识别与翻译面板真实交互', () => {
  it('单文本取消后不显示迟到的成功响应', async () => {
    let finish!: (result: unknown) => void
    api.translateText.mockReturnValue(new Promise(resolve => { finish = resolve }))
    api.cancelAiJob.mockResolvedValueOnce(undefined)
    await mount('TranslationPanel')
    findNode(root, node => node.type === 'button' && content(node).trim() === '翻译')!.props.onClick()
    await flushComponent()
    findNode(root, node => node.type === 'button' && content(node).trim() === '取消翻译')!.props.onClick()
    await flushComponent()
    finish({ requestId: api.translateText.mock.calls[0][0], translatedText: 'late translation', targetLanguage: 'zh-CN' })
    await flushComponent()
    expect(findNode(root, node => node.type === 'textarea' && node.props.id === 'translation-output')?.value).toBe('')
  })
  it('DeepL 无模型也能翻译；确认前不上传，卸载取消当前批次', async () => {
    const settings = { ...defaultAiSettings(), provider: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate' }
    api.aiTranslationConfig.mockResolvedValue(settings)
    api.translateTexts.mockReturnValue(new Promise(() => {}))
    await mount('ImageTranslation')
    const button = () => findNode(root, node => node.type === 'button' && content(node).trim() === '翻译')!
    expect(button().props.disabled).toBe(true)
    expect(api.translateTexts).not.toHaveBeenCalled()
    findNode(root, node => node.type === 'input' && node.props.type === 'checkbox')!.props['onUpdate:modelValue'](true)
    await flushComponent()
    expect(button().props.disabled).toBe(false)
    button().props.onClick(); await flushComponent()
    expect(api.translateTexts).toHaveBeenCalledExactlyOnceWith(expect.any(String), { texts: ['hello'], sourceLanguage: null, targetLanguage: 'zh-CN' }, true, settings)
    expect(api.translateText).not.toHaveBeenCalled()
    const requestId = api.translateTexts.mock.calls[0][0]
    memoryRenderer.render(null, root); await flushComponent()
    expect(api.cancelAiJob).toHaveBeenCalledExactlyOnceWith(requestId)
  })
  it('贴图使用截图的可编辑结果卡，复制用户修正后的文字', async () => {
    api.recognizePinText.mockResolvedValueOnce({ ...recognition, fullText: '识别原文' })
    await mount('PinRecognition')
    const input = findNode(root, node => node.type === 'textarea')!
    expect(input.props['aria-label']).toBe('识别文字，可编辑')
    expect(input.props.readonly).toBeUndefined()
    input.props['onUpdate:modelValue']('修正后的原文'); await flushComponent()
    const copy = findNode(root, node => node.type === 'button' && content(node).trim() === '复制文字')!
    copy.props.onClick(); await flushComponent()
    expect(api.copyOcrText).toHaveBeenCalledExactlyOnceWith('修正后的原文')
  })
  it('贴图将编辑草稿送入共享翻译流程，返回识别不重复 OCR 或丢失草稿', async () => {
    api.recognizePinText.mockResolvedValueOnce({ ...recognition, fullText: '识别原文' })
    await mount('PinRecognition')
    findNode(root, node => node.type === 'textarea')!.props['onUpdate:modelValue']('修正后的原文')
    await flushComponent()
    findNode(root, node => node.type === 'button' && content(node).trim() === '翻译文字')!.props.onClick()
    await flushComponent()
    expect(content(root)).toContain('修正后的原文')
    expect(api.translateText).not.toHaveBeenCalled()
    findNode(root, node => node.type === 'button' && content(node).trim() === '翻译')!.props.onClick()
    await flushComponent()
    expect(api.translateText).toHaveBeenCalledOnce()
    expect(api.translateText.mock.calls[0][1]).toBe('修正后的原文')
    findNode(root, node => node.type === 'button' && content(node).trim() === '返回识别')!.props.onClick()
    await flushComponent()
    expect(findNode(root, node => node.type === 'textarea')!.value).toBe('修正后的原文')
    expect(api.recognizePinText).toHaveBeenCalledOnce()
  })
  it('贴图直接翻译入口仍可返回同一份可编辑识别结果', async () => {
    api.recognizePinText.mockResolvedValueOnce({ ...recognition, fullText: '直接翻译的原文' })
    await mount('PinRecognition', { mode: 'translate' })
    expect(content(root)).toContain('直接翻译的原文')
    expect(api.translateText).not.toHaveBeenCalled()
    findNode(root, node => node.type === 'button' && content(node).trim() === '返回识别')!.props.onClick()
    await flushComponent()
    expect(findNode(root, node => node.type === 'textarea')!.value).toBe('直接翻译的原文')
    expect(api.recognizePinText).toHaveBeenCalledOnce()
  })
  it('贴图空识别结果与截图一致，允许手动输入后复制或翻译', async () => {
    api.recognizePinText.mockResolvedValueOnce({ ...recognition, fullText: '' })
    await mount('PinRecognition')
    const input = findNode(root, node => node.type === 'textarea')!
    const copy = () => findNode(root, node => node.type === 'button' && content(node).trim() === '复制文字')!
    expect(input.value).toBe('')
    expect(copy().props.disabled).toBe(true)
    input.props['onUpdate:modelValue']('补充文字'); await flushComponent()
    expect(copy().props.disabled).toBe(false)
    copy().props.onClick(); await flushComponent()
    expect(api.copyOcrText).toHaveBeenCalledExactlyOnceWith('补充文字')
  })
  for (const name of Object.keys(components) as (keyof typeof components)[]) {
    it(`${name} 处理中且文本框聚焦时仍能关闭`, async () => {
      const panel = await mount(name), event = escape()
      panel.props.onKeydown(event)
      expect(close).toHaveBeenCalledExactlyOnceWith()
      expect(event.defaultPrevented).toBe(true)
    })
    it.each([{ isComposing: true }, { keyCode: 229 }, { repeat: true }, { defaultPrevented: true }])(`${name} 保留输入法和内层事件保护：%o`, async patch => {
      const panel = await mount(name)
      panel.props.onKeydown(escape(patch))
      expect(close).not.toHaveBeenCalled()
    })
  }
  it('关闭后迟到的 OCR 结果不能污染重新打开的面板', async () => {
    let finish!: (value: unknown) => void
    api.recognizePinText.mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
    await mount('PinRecognition'); memoryRenderer.render(null, root)
    api.recognizePinText.mockResolvedValueOnce({ ...recognition, fullText: 'fresh', lines: [{ id: 'new', text: 'fresh', order: 0 }] })
    await mount('PinRecognition')
    finish({ ...recognition, fullText: 'late', lines: [{ id: 'old', text: 'late', order: 0 }] }); await flushComponent()
    expect(findNode(root, node => node.type === 'textarea')?.value).toBe('fresh')
  })
  it.each(['ImageTranslation', 'TranslationPanel'] as const)('%s 在真实翻译期间卸载会取消请求，取消失败也能正常退出', async name => {
    await mount(name)
    const translate = findNode(root, node => node.type === 'button' && content(node).trim() === '翻译')!
    expect(translate.props.disabled).toBeFalsy()
    translate.props.onClick(); await flushComponent()
    expect(api.translateText).toHaveBeenCalledOnce()
    const requestId = api.translateText.mock.calls[0][0]
    memoryRenderer.render(null, root); await flushComponent()
    expect(api.cancelAiJob).toHaveBeenCalledExactlyOnceWith(requestId)
  })
})

// 窗口级兜底仍覆盖整个截图/贴图入口；面板内部测试不依赖这些私有实现。
function script(file: string) {
  const domain = file === 'CaptureOverlay' ? 'capture' : file === 'PinWindow' ? 'pins' : 'recognition'
  const source = parse(readFileSync(`src/features/${domain}/components/${file}.vue`, 'utf8')).descriptor.scriptSetup!.content
  return ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true)
}
function transpile(source: string) {
  return ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
}
function declarations(ast: ts.SourceFile, names: string[]) {
  return ast.statements.filter(node => ts.isFunctionDeclaration(node) && names.includes(node.name?.text ?? ''))
    .map(node => node.getText(ast)).join('\n')
}
function handler(file: string, context: Record<string, unknown>, names = ['key']) {
  const ast = script(file)
  return new Function(...Object.keys(context), `${transpile(declarations(ast, names))}; return key`)(...Object.values(context))
}

describe('窗口识别面板 Esc 兜底', () => {
  it('焦点落在截图外层时，第一次 Esc 只关识别面板', () => {
    const ocrResult = { value: {} as object | null }, closeOcr = vi.fn(() => { ocrResult.value = null })
    const cancel = vi.fn()
    const key = handler('CaptureOverlay', {
      handleEscape, liveRecordingId: { value: null }, qrResult: { value: null }, ocrResult, closeOcr, cancel,
    })
    const event = escape()
    key(event)
    expect(closeOcr).toHaveBeenCalledOnce()
    expect(cancel).not.toHaveBeenCalled()
    // 子面板消费后，窗口入口也不能继续关闭截图。
    key(event)
    expect(cancel).not.toHaveBeenCalled()
  })
  it('焦点落在贴图外层时，Esc 只关图片翻译面板', () => {
    const recognitionMode = { value: 'translate' as string | null }
    const closeRecognition = vi.fn(() => { recognitionMode.value = null }), action = vi.fn()
    const key = handler('PinWindow', { handleEscape, contextMenu: { value: null }, recognitionMode, closeRecognition, action })
    const event = escape()
    key(event)
    expect(closeRecognition).toHaveBeenCalledOnce()
    key(event)
    expect(action).not.toHaveBeenCalled()
  })
})
