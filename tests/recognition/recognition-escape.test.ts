import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, type Component } from 'vue'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { webcrypto } from 'node:crypto'
import { handleEscape } from '../../src/shared/ui/escape'
import { defaultAiSettings } from '../../src/features/recognition/ai'
import { clientComponent, MemoryNode, memoryRenderer, findNode, flushComponent } from '../helpers/component-host'
const api = vi.hoisted(() => ({ recognizePinText: vi.fn(), recognizePinQr: vi.fn(), aiTranslationConfig: vi.fn(), translateText: vi.fn(), cancelAiJob: vi.fn() }))
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
async function mount(name: keyof typeof components) {
  const props = { onClose: close, text: 'hello', frame, mode: 'ocr', recognition, image: { dataUrl: '', width: 640, height: 480 }, captureResult: { id: 7, items: [] } }
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
  api.recognizePinText.mockReturnValue(new Promise(() => {}))
  api.aiTranslationConfig.mockResolvedValue(defaultAiSettings())
  api.translateText.mockReturnValue(new Promise(() => {}))
  api.cancelAiJob.mockRejectedValue(new Error('already finished'))
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

describe('识别与翻译面板真实交互', () => {
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
    expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe('fresh')
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
