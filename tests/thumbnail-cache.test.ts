import { describe, expect, it, vi } from 'vitest'
import { createThumbnailCache } from '../src/lib/thumbnail-cache'

const signal = () => new AbortController().signal
describe('缩略图缓存与请求调度', () => {
  it('bounds characters and entries while retaining recently used thumbnails', async () => {
    const load = vi.fn(async (id: number) => String(id).repeat(4))
    const cache = createThumbnailCache(load, 8, 2)
    await cache.get(1, signal()); await cache.get(2, signal()); await cache.get(1, signal())
    await cache.get(3, signal()); await cache.get(1, signal())
    expect(load).toHaveBeenCalledTimes(3)
    await cache.get(2, signal())
    expect(load).toHaveBeenCalledTimes(4)
    const small = createThumbnailCache(load, 2, 10)
    await small.get(4, signal()); await small.get(4, signal())
    expect(load).toHaveBeenCalledTimes(6)
  })
  it('cancels queued requests and bounds active IPC while discarding departed rows', async () => {
    const completions = new Map<number, (value: string) => void>()
    const load = vi.fn((id: number) => new Promise<string>(resolve => completions.set(id, resolve)))
    const cache = createThumbnailCache(load, 100, 10, 1)
    const first = new AbortController(), queued = new AbortController()
    const one = cache.get(1, first.signal).catch(error => error.name)
    const two = cache.get(2, queued.signal).catch(error => error.name)
    const three = cache.get(3, signal())
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1))
    queued.abort(); first.abort()
    expect(await two).toBe('AbortError'); expect(await one).toBe('AbortError')
    expect(load).toHaveBeenCalledTimes(1)
    completions.get(1)!('old')
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2))
    expect(load.mock.calls.map(call => call[0])).toEqual([1, 3])
    completions.get(3)!('new'); expect(await three).toBe('new')
    expect(await cache.get(3, signal())).toBe('new')
  })
  it('does not repopulate cleared history from an in-flight response and recovers after errors', async () => {
    let finish!: (value: string) => void
    const load = vi.fn().mockImplementationOnce(() => new Promise<string>(resolve => { finish = resolve }))
      .mockRejectedValueOnce(new Error('missing')).mockResolvedValue('fresh')
    const cache = createThumbnailCache(load)
    const pending = cache.get(1, signal())
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1))
    cache.clear(); finish('old'); expect(await pending).toBe('old')
    await expect(cache.get(1, signal())).rejects.toThrow('missing')
    expect(await cache.get(1, signal())).toBe('fresh')
    expect(load).toHaveBeenCalledTimes(3)
  })
})
