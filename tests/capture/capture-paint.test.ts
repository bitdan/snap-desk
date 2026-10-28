import { afterEach, describe, expect, it, vi } from 'vitest'
import { presentCaptureImage, waitForCapturePaint } from '../../src/features/capture/capture-paint'

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals() })
function frames() {
  vi.useFakeTimers()
  const callbacks = new Map<number, FrameRequestCallback>()
  let next = 0
  vi.stubGlobal('requestAnimationFrame', vi.fn(callback => { callbacks.set(++next, callback); return next }))
  vi.stubGlobal('cancelAnimationFrame', vi.fn(id => callbacks.delete(id)))
  return async () => {
    for (let i = 0; i < 2; i++) {
      const pending = [...callbacks.entries()]; callbacks.clear()
      for (const [, callback] of pending) callback(performance.now())
      await Promise.resolve()
    }
  }
}
function fixture() {
  let decode!: () => void
  let current = true
  const options = {
    image: { decode: vi.fn(() => new Promise<void>(resolve => { decode = resolve })) },
    receivedAt: performance.now(), isCurrent: () => current,
    decoded: vi.fn(), decodeFailed: vi.fn(async () => {}), ready: vi.fn(async () => {}), painted: vi.fn(async () => {}),
  }
  return { options, start: () => presentCaptureImage(options), decode: () => decode(), cancel: () => { current = false } }
}

describe('截图首帧时序', () => {
  it('两次 rAF 成功时标记绘制回调；隐藏窗口超时单独标记且清除回调', async () => {
    const paint = frames()
    const visible = waitForCapturePaint()
    await paint()
    expect((await visible).source).toBe('raf')
    const hidden = waitForCapturePaint()
    await vi.advanceTimersByTimeAsync(100)
    expect((await hidden).source).toBe('timeout')
    expect(vi.getTimerCount()).toBe(0)
  })
  it('解码和预合成结束后显示，再等待可见绘制后发送计时回执', async () => {
    const paint = frames(), f = fixture(), pending = f.start()
    expect(f.options.ready).not.toHaveBeenCalled()
    f.decode(); await vi.advanceTimersByTimeAsync(0)
    expect(f.options.decoded).toHaveBeenCalledOnce()
    expect(f.options.ready).not.toHaveBeenCalled()
    await paint(); await vi.advanceTimersByTimeAsync(0)
    expect(f.options.ready).toHaveBeenCalledOnce()
    expect(f.options.painted).not.toHaveBeenCalled()
    await paint(); await pending
    expect(f.options.painted).toHaveBeenCalledWith(expect.objectContaining({ paintSource: 'raf' }))
  })
  it.each(['decode', 'pre-paint', 'show', 'post-paint'] as const)('在 %s 阶段取消或换图，丢弃旧图片的显示或计时回执', async stage => {
    const paint = frames(), f = fixture(), pending = f.start()
    if (stage === 'decode') f.cancel()
    f.decode(); await vi.advanceTimersByTimeAsync(0)
    if (stage === 'pre-paint') f.cancel()
    if (stage === 'show') f.options.ready.mockImplementationOnce(async () => { f.cancel() })
    await paint(); await vi.advanceTimersByTimeAsync(0)
    if (stage === 'post-paint') f.cancel()
    await paint(); await pending
    expect(f.options.painted).not.toHaveBeenCalled()
    if (stage === 'decode' || stage === 'pre-paint') expect(f.options.ready).not.toHaveBeenCalled()
  })
  it('解码失败保留既有失败处理，不显示窗口；诊断失败不影响显示', async () => {
    const paint = frames(), failed = fixture()
    failed.options.image.decode.mockRejectedValueOnce(new Error('decode failed'))
    await failed.start()
    expect(failed.options.decodeFailed).toHaveBeenCalledOnce()
    expect(failed.options.ready).not.toHaveBeenCalled()
    const f = fixture(), pending = f.start()
    f.options.painted.mockRejectedValueOnce(new Error('diagnostics unavailable'))
    f.decode(); await vi.advanceTimersByTimeAsync(0); await paint(); await vi.advanceTimersByTimeAsync(0); await paint()
    await expect(pending).resolves.toBeUndefined()
    expect(f.options.ready).toHaveBeenCalledOnce()
  })
})
