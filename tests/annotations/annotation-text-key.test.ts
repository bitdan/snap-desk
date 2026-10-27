import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'

// 执行真实文字框按键入口，检查局部按键不会继续冒泡触发全局退出。
const script = parse(readFileSync('src/features/annotations/components/AnnotationSurface.vue', 'utf8')).descriptor.scriptSetup!.content
const ast = ts.createSourceFile('surface.ts', script, ts.ScriptTarget.Latest, true)
const entry = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'textKey')!
const code = ts.transpileModule(entry.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
describe('文字框局部 Esc', () => {
  it.each(['Escape', 'Enter'])('%s 提交文字并阻止全局退出', key => {
    const finish = vi.fn()
    const handler = new Function('finishText', `${code}; return textKey`)(finish)
    const event = { key, stopPropagation: vi.fn(), preventDefault: vi.fn() }
    handler(event)
    expect(finish).toHaveBeenCalledOnce()
    expect(event.stopPropagation).toHaveBeenCalledOnce()
    expect(event.preventDefault).toHaveBeenCalledOnce()
  })
  it.each([{ isComposing: true }, { keyCode: 229 }, { repeat: true }])('组词或长按不重复提交：%o', patch => {
    const finish = vi.fn(), handler = new Function('finishText', `${code}; return textKey`)(finish)
    const event = { key: 'Escape', stopPropagation: vi.fn(), preventDefault: vi.fn(), ...patch }
    handler(event)
    expect(finish).not.toHaveBeenCalled()
    expect(event.stopPropagation).toHaveBeenCalledOnce()
  })
})
