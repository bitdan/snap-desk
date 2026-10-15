import { describe, expect, it } from 'vitest'
import { canEditPin, minimumZoom, pinDisplaySize, stepZoom } from '../src/lib/pins'

describe('pin display controls', () => {
  it('100% maps selected image pixels to screen pixels at Windows display scales', () => {
    for (const pixelRatio of [1, 1.25, 1.5, 2]) {
      const size = pinDisplaySize(1063, 788, 1, 1600, 1200, true, pixelRatio)
      expect(size.imageWidth * pixelRatio).toBeCloseTo(1063)
      expect(size.imageHeight * pixelRatio).toBeCloseTo(788)
      expect(size.stageWidth).toBeCloseTo(1063 / pixelRatio + 2)
      expect(size.stageHeight).toBeCloseTo(788 / pixelRatio + 2)
      const tiny = pinDisplaySize(2, 1, 1, 266, 290, true, pixelRatio)
      expect(tiny.imageHeight).toBe(20)
    }
  })
  it('keeps the complete image border inside a shrinking viewport before the zoom reply arrives', () => {
    for (const expanded of [true, false]) {
      for (const zoom of [1, 0.5, 0.1]) {
        const size = pinDisplaySize(1116, 558, zoom, 266, 290, expanded)
        expect(size.stageWidth).toBeLessThanOrEqual(242)
        expect(size.stageHeight).toBeLessThanOrEqual(expanded ? 162 : 222)
        expect(size.stageWidth).toBeGreaterThanOrEqual(22)
        expect(size.stageHeight).toBeGreaterThanOrEqual(22)
      }
    }
    const tiny = pinDisplaySize(2, 1, 0.1, 266, 290, true)
    expect(tiny).toEqual({ imageWidth: 40, imageHeight: 20, stageWidth: 42, stageHeight: 22 })
  })
  it('keeps small and narrow images visible after repeated shrinking', () => {
    for (const [width, height, minimum] of [[200, 100, 0.2], [100, 200, 0.2], [20, 300, 1], [800, 600, 0.1], [2, 300, 10], [300, 1, 20], [19, 19, 20 / 19]]) {
      expect(minimumZoom(width, height)).toBe(minimum)
      let zoom = 1
      for (let i = 0; i < 100; i++) zoom = stepZoom(zoom, -1, minimum)
      expect(zoom).toBe(minimum)
      expect(width * zoom).toBeGreaterThanOrEqual(20)
      expect(height * zoom).toBeGreaterThanOrEqual(20)
      expect(stepZoom(zoom, 1, minimum)).toBeGreaterThanOrEqual(minimum)
    }
  })
  it('clamps repeated wheel zoom and avoids accumulated decimal drift', () => {
    let zoom = 1
    for (let i = 0; i < 50; i++) zoom = stepZoom(zoom, 1)
    expect(zoom).toBe(4)
    for (let i = 0; i < 50; i++) zoom = stepZoom(zoom, -1)
    expect(zoom).toBe(0.1)
    for (let i = 0; i < 9; i++) zoom = stepZoom(zoom, 1)
    expect(zoom).toBe(1)
    expect(stepZoom(0.375, 1)).toBe(0.5)
  })
  it('blocks edits before load, during native work, and while locked', () => {
    const view = { zoom: 1, opacity: 1, topmost: true, locked: false, toolbarExpanded: true }
    expect(canEditPin(undefined, false)).toBe(false)
    expect(canEditPin(view, true)).toBe(false)
    expect(canEditPin({ ...view, locked: true }, false)).toBe(false)
    expect(canEditPin(view, false)).toBe(true)
  })
})
