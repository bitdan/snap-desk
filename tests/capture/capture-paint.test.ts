import { afterEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { waitForCapturePaint } from '../../src/features/capture/capture-paint'

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals() })

describe('隐藏截图窗口首帧', () => {
  it('等待两次绘制回调，隐藏窗口不派发回调时也能结束等待', async () => {
    vi.useFakeTimers()
    const callbacks: FrameRequestCallback[] = []
    vi.stubGlobal('requestAnimationFrame', vi.fn(callback => callbacks.push(callback)))
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
    const done = vi.fn()
    const painted = waitForCapturePaint().then(done)
    callbacks.shift()!(0)
    await Promise.resolve()
    expect(done).not.toHaveBeenCalled()
    callbacks.shift()!(16)
    await painted
    expect(done).toHaveBeenCalledTimes(1)
    const hidden = waitForCapturePaint().then(done)
    await vi.advanceTimersByTimeAsync(100)
    await hidden
    expect(done).toHaveBeenCalledTimes(2)
    expect(vi.getTimerCount()).toBe(0)
  })

  // 执行组件的真实加载入口，用可控的解码/绘制 Promise 重放首次截屏和取消时序。
  const script = parse(readFileSync('src/features/capture/components/CaptureOverlay.vue', 'utf8')).descriptor.scriptSetup!.content
  const ast = ts.createSourceFile('capture.ts', script, ts.ScriptTarget.Latest, true)
  const entry = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'loaded')!
  const code = ts.transpileModule(entry.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
  function fixture() {
    let decoded!: () => void, painted!: () => void
    const image = { decode: vi.fn(() => new Promise<void>(resolve => { decoded = resolve })) }
    const context = {
      frame: { value: { id: 7 } as { id: number } | null }, screenImage: { value: image },
      imageUrl: { value: '/7/0.png' }, disposed: false, imageReady: { value: false },
      error: { value: '' }, api: { ready: vi.fn() }, imageFailed: vi.fn(), nextTick: vi.fn(),
      waitForCapturePaint: vi.fn(() => new Promise<void>(resolve => { painted = resolve })),
    }
    const loaded = new Function(...Object.keys(context), `${code}; return loaded`)(...Object.values(context))
    return { context, load: () => loaded({ target: image }), decode: () => decoded(), paint: () => painted() }
  }
  it('解码和绘制等待完成前不显示原生窗口', async () => {
    const f = fixture(), pending = f.load()
    expect(f.context.api.ready).not.toHaveBeenCalled()
    f.decode()
    await vi.waitFor(() => expect(f.context.waitForCapturePaint).toHaveBeenCalled())
    expect(f.context.api.ready).not.toHaveBeenCalled()
    f.paint(); await pending
    expect(f.context.api.ready).toHaveBeenCalledExactlyOnceWith(7)
  })
  it.each(['cancel', 'replace'] as const)('等待期间 %s 不重新唤起旧窗口', async action => {
    const f = fixture(), pending = f.load()
    f.decode()
    await vi.waitFor(() => expect(f.context.waitForCapturePaint).toHaveBeenCalled())
    if (action === 'cancel') f.context.frame.value = null
    else f.context.imageUrl.value = '/7/1.png'
    f.paint(); await pending
    expect(f.context.api.ready).not.toHaveBeenCalled()
  })
})
