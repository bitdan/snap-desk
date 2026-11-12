import { afterEach, describe, expect, it, vi } from 'vitest'
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string, scheme: string) => `http://${scheme}.localhost/${path}` }))
import { defaultPinAppearance } from '../../src/features/pins/pins'
import { usePinImage } from '../../src/features/pins/pin-image'
import type { PinFrame } from '../../src/features/pins/types'

function frame(revision = 0): PinFrame {
  return { label: 'pin-1', revision, width: 20, height: 30, view: { zoom: 1, opacity: 1, locked: false, topmost: true, toolbarExpanded: true }, appearance: defaultPinAppearance(), transparencySupported: true, annotations: [] }
}
const url = (revision: number) => `http://snap-image.localhost/pin/pin-1/${revision}.png`
afterEach(() => vi.useRealTimers())
describe('贴图版本地址与隐藏释放', () => {
  it('底图地址由标签与像素版本决定，同版本只更新元信息', async () => {
    const fetch = vi.fn().mockResolvedValueOnce(frame()).mockResolvedValueOnce({ ...frame(), width: 40 })
    const image = usePinImage(fetch)
    expect(await image.refresh()).toBe(true)
    expect(await image.refresh()).toBe(false)
    expect(image.frame.value?.src).toBe(url(0))
    expect(image.frame.value?.width).toBe(40)
    image.dispose()
  })
  it('长期隐藏释放底图地址，重开时按当前版本恢复', async () => {
    vi.useFakeTimers()
    const fetch = vi.fn().mockResolvedValue(frame()), image = usePinImage(fetch)
    await image.refresh(); image.visibility(false)
    await vi.advanceTimersByTimeAsync(4999)
    expect(image.frame.value?.src).toBe(url(0))
    await vi.advanceTimersByTimeAsync(1)
    expect(image.frame.value?.src).toBe('')
    image.visibility(true); await image.refresh()
    expect(image.frame.value?.src).toBe(url(0))
    image.dispose()
  })
  it('短暂隐藏复用图片，旧释放定时器不清掉已经显示的图片', async () => {
    vi.useFakeTimers()
    const image = usePinImage(vi.fn().mockResolvedValue(frame()))
    await image.refresh(); image.visibility(false); await vi.advanceTimersByTimeAsync(100)
    image.visibility(true); await image.refresh(); await vi.advanceTimersByTimeAsync(5000)
    expect(image.frame.value?.src).toBe(url(0))
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
  it('新版本替换图片，旧版本响应不能覆盖它', () => {
    const image = usePinImage(vi.fn())
    image.replace(frame(1))
    expect(image.replace(frame(0))).toBe(false)
    expect(image.frame.value?.src).toBe(url(1))
    image.dispose()
  })
  it('新请求完成后，旧请求的图片与错误都不能覆盖当前状态', async () => {
    let finish!: (value: PinFrame) => void, fail!: (error: Error) => void
    const fetch = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject }))
      .mockResolvedValue(frame(2))
    const image = usePinImage(fetch), first = image.refresh(), second = image.refresh()
    await image.refresh(); finish(frame(1)); fail(new Error('late failure'))
    expect(await first).toBe(false); expect(await second).toBe(false)
    expect(image.frame.value?.src).toBe(url(2))
    image.dispose()
  })
})
