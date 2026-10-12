import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseRecentSelection, readRecentSelection, restoreRecentSelection, writeRecentSelection } from '../src/lib/recent-selection'
import { toPixels } from '../src/lib/selection'

const image = { width: 2560, height: 1440 }
const rect = { x: 133, y: 97, width: 701, height: 503 }
const saved = { version: 1 as const, image, rect }
afterEach(() => vi.unstubAllGlobals())

describe('最近选区', () => {
  it('在不同轴缩放下精确恢复原图像素范围', () => {
    for (const viewport of [{ width: 1707, height: 960 }, { width: 1280, height: 900 }]) {
      const restored = restoreRecentSelection(saved, image, viewport)!
      expect(toPixels(restored, viewport, image)).toEqual(rect)
    }
  })
  it('分辨率变化或窗口尺寸无效时不恢复', () => {
    expect(restoreRecentSelection(saved, { ...image, width: 1920 }, image)).toBeNull()
    expect(restoreRecentSelection(saved, image, { width: 0, height: 900 })).toBeNull()
    expect(restoreRecentSelection(saved, image, { width: Infinity, height: 900 })).toBeNull()
  })
  it('丢弃损坏、旧版本、越界、空白和非整数记录', () => {
    for (const value of [null, '{', '{}', JSON.stringify({ ...saved, version: 2 }),
      ...[{ ...rect, x: -1 }, { ...rect, x: '1' }, { ...rect, width: 0 }, { ...rect, y: 0.5 },
        { ...rect, width: 2560 }, { ...rect, height: 1440 }].map(rect => JSON.stringify({ ...saved, rect }))]) {
      expect(parseRecentSelection(value)).toBeNull()
    }
  })
  it('支持全屏和右下角单像素范围', () => {
    for (const rect of [{ x: 0, y: 0, ...image }, { x: 2559, y: 1439, width: 1, height: 1 }]) {
      expect(parseRecentSelection(JSON.stringify({ ...saved, rect }))?.rect).toEqual(rect)
    }
  })
  it('存储仅包含几何信息，可跨窗口重建读取', () => {
    const storage = new Map<string, string>()
    vi.stubGlobal('localStorage', { setItem: (key: string, value: string) => storage.set(key, value), getItem: (key: string) => storage.get(key) ?? null })
    const frame = { ...image, id: 12, dataUrl: 'private-image' }
    expect(writeRecentSelection(rect, frame)).toEqual(saved)
    expect(readRecentSelection()).toEqual(saved)
    expect([...storage.values()][0]).not.toContain('private-image')
  })
  it('存储不可用时保留内存结果且不抛异常', () => {
    vi.stubGlobal('localStorage', { setItem: () => { throw new Error('quota') }, getItem: () => { throw new Error('denied') } })
    expect(readRecentSelection()).toBeNull()
    expect(writeRecentSelection(rect, image)).toEqual(saved)
  })
})
