import { afterEach, describe, expect, it, vi } from 'vitest'
import { usePinImage } from '../../src/features/pins/pin-image'
import type { PinFrame } from '../../src/features/pins/types'

function frame(revision = 0, dataUrl = 'data:image/png;base64,image'): PinFrame {
  return { label: 'pin-1', revision, width: 20, height: 30, dataUrl, view: { zoom: 1, opacity: 1, locked: false, topmost: true, toolbarExpanded: true }, transparencySupported: true, annotations: [] }
}
afterEach(() => vi.useRealTimers())
describe('贴图版本缓存与隐藏释放', () => {
  it('相同版本只读元信息，底图不重新替换', async () => {
    const fetch = vi.fn().mockResolvedValueOnce(frame()).mockResolvedValueOnce({ ...frame(0, ''), width: 40 })
    const image = usePinImage(fetch)
    expect(await image.refresh()).toBe(true)
    expect(await image.refresh()).toBe(false)
    expect(fetch.mock.calls).toEqual([[null], [0]])
    expect(image.frame.value?.dataUrl).toBe(frame().dataUrl)
    expect(image.frame.value?.width).toBe(40)
    image.dispose()
  })
  it('长期隐藏释放 Data URL，重开时完整加载同一版本', async () => {
    vi.useFakeTimers()
    const fetch = vi.fn().mockResolvedValue(frame()), image = usePinImage(fetch)
    await image.refresh(); image.visibility(false)
    await vi.advanceTimersByTimeAsync(4999)
    expect(image.frame.value?.dataUrl).toBeTruthy()
    await vi.advanceTimersByTimeAsync(1)
    expect(image.frame.value?.dataUrl).toBe('')
    image.visibility(true); await image.refresh()
    expect(fetch).toHaveBeenLastCalledWith(null)
    expect(image.frame.value?.dataUrl).toBeTruthy()
    image.dispose()
  })
  it('短暂隐藏复用图片，旧释放定时器不清掉已经显示的图片', async () => {
    vi.useFakeTimers()
    const fetch = vi.fn().mockResolvedValueOnce(frame()).mockResolvedValue(frame(0, '')), image = usePinImage(fetch)
    await image.refresh(); image.visibility(false); await vi.advanceTimersByTimeAsync(100)
    image.visibility(true); await image.refresh(); await vi.advanceTimersByTimeAsync(5000)
    expect(fetch).toHaveBeenLastCalledWith(0)
    expect(image.frame.value?.dataUrl).toBeTruthy()
    image.dispose()
  })
  it.each(['hide', 'dispose'])('%s 后迟到的读取结果不能重新驻留图片', async action => {
    let finish!: (frame: PinFrame) => void
    const image = usePinImage(() => new Promise(resolve => { finish = resolve }))
    const pending = image.refresh()
    if (action === 'hide') image.visibility(false); else image.dispose()
    finish(frame()); expect(await pending).toBe(false)
    expect(image.frame.value).toBeNull()
    image.dispose()
  })
  it('新版本替换图片，旧版本和失去缓存的空图片响应不能覆盖它', async () => {
    const image = usePinImage(vi.fn())
    image.replace(frame(1, 'new'))
    expect(image.replace(frame(0, 'old'))).toBe(false)
    expect(image.frame.value?.dataUrl).toBe('new')
    expect(() => image.replace(frame(2, ''))).toThrow()
    image.dispose()
  })
  it('新请求完成后，旧请求的图片与错误都不能覆盖当前状态', async () => {
    let finish!: (value: PinFrame) => void, fail!: (error: Error) => void
    const fetch = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject }))
      .mockResolvedValue(frame(2, 'latest'))
    const image = usePinImage(fetch), first = image.refresh(), second = image.refresh()
    await image.refresh(); finish(frame(1, 'old')); fail(new Error('late failure'))
    expect(await first).toBe(false); expect(await second).toBe(false)
    expect(image.frame.value?.dataUrl).toBe('latest')
    image.dispose()
  })
})
