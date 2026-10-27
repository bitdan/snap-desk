import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { handleEscape } from '../src/lib/escape'

// 执行组件真实的键盘入口，重放输入框焦点、忙碌和嵌套面板事件，不依赖桌面窗口。
function script(file: string) {
  const source = parse(readFileSync(`src/components/${file}.vue`, 'utf8')).descriptor.scriptSetup!.content
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
function escape(patch: Record<string, unknown> = {}) {
  const event = {
    key: 'Escape', defaultPrevented: false, isComposing: false, keyCode: 27, repeat: false,
    target: { tagName: 'TEXTAREA' }, preventDefault: vi.fn(() => { event.defaultPrevented = true }), ...patch,
  }
  return event as unknown as KeyboardEvent
}

describe('识别与翻译面板 Esc', () => {
  for (const file of ['PinRecognition', 'ImageTranslation', 'TranslationPanel', 'OcrResultCard', 'QrResultCard']) {
    function fixture() {
      const emit = vi.fn(), dismissed = { value: false }
      const key = handler(file, { handleEscape, emit, busy: { value: true }, dismissed }, ['key', 'close'])
      return { emit, key, dismissed }
    }
    it(`${file} 在处理中、文本框聚焦时也能退出`, () => {
      const f = fixture(), event = escape()
      f.key(event)
      expect(f.emit).toHaveBeenCalledExactlyOnceWith('close')
      expect(event.defaultPrevented).toBe(true)
    })
    it.each([{ isComposing: true }, { keyCode: 229 }, { repeat: true }, { defaultPrevented: true }])(
      `${file} 保留输入法、长按与内层处理保护：%o`, patch => {
        const f = fixture()
        f.key(escape(patch))
        expect(f.emit).not.toHaveBeenCalled()
      },
    )
  }
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
  it('关闭图片识别后，迟到的 OCR 结果不再回写面板', async () => {
    const ast = script('PinRecognition')
    const hook = ast.statements.find(node => ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
      && node.expression.expression.getText(ast) === 'onUnmounted')!
    let cleanup!: () => void, finish!: (value: unknown) => void
    const ocr = { value: null }, selected = { value: [] }
    const context = {
      disposed: false, busy: { value: false }, error: { value: '' }, message: { value: '' }, pendingLink: { value: null },
      props: { mode: 'ocr' }, ocr, selected,
      api: { recognizePinText: vi.fn(() => new Promise(resolve => { finish = resolve })) },
      onUnmounted: (callback: () => void) => { cleanup = callback },
    }
    const recognize = new Function(...Object.keys(context),
      `${transpile(`${declarations(ast, ['run', 'recognize'])}\n${hook.getText(ast)}`)}; return recognize`)(...Object.values(context))
    const pending = recognize()
    cleanup()
    finish({ lines: [{ id: 'late-result' }] })
    await pending
    expect(ocr.value).toBeNull()
    expect(selected.value).toEqual([])
  })
  it.each(['ImageTranslation', 'TranslationPanel'])('%s 卸载时取消在途翻译，取消失败也不打断退出', async file => {
    const ast = script(file)
    const hook = ast.statements.find(node => ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
      && node.expression.expression.getText(ast) === 'onUnmounted')!
    let cleanup!: () => void
    const api = { cancelAiJob: vi.fn().mockRejectedValue(new Error('already finished')) }
    const context = { api, requestId: 'pending-translation', disposed: false, cancelled: false, observer: null,
      error: { value: '' }, onUnmounted: (callback: () => void) => { cleanup = callback } }
    new Function(...Object.keys(context), transpile(`${declarations(ast, ['cancel'])}\n${hook.getText(ast)}`))(...Object.values(context))
    cleanup()
    await Promise.resolve()
    expect(api.cancelAiJob).toHaveBeenCalledExactlyOnceWith('pending-translation')
    expect(context.error.value).toBe('')
  })
})
