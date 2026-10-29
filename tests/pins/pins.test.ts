import { describe, expect, it } from 'vitest'
import layout from '../../src/features/pins/pin-layout.json'
import { defaultPinAppearance, pinAppearanceStyle, validatePinAppearance, canEditPin, minimumZoom, pinDisplaySize, pinImagePadding, stepZoom } from '../../src/features/pins/pins'

describe('pin display controls', () => {
  it('matches the native image inset at fractional DPI without shifting the first image pixel', () => {
    for (const pixelRatio of [1, 1.25, 1.5, 1.75, 2]) {
      const inset = (pinImagePadding(pixelRatio) + layout.border) * pixelRatio
      expect(inset).toBeCloseTo(Math.round((layout.padding + layout.border) * pixelRatio))
      const viewportWidth = (1233 + Math.round(inset) * 2) / pixelRatio
      const viewportHeight = (626 + Math.round(inset) * 2) / pixelRatio + layout.gap + layout.expandedHeight
      const size = pinDisplaySize(1233, 626, 1, viewportWidth, viewportHeight, true, pixelRatio)
      expect(size.imageWidth * pixelRatio).toBeCloseTo(1233)
      expect(size.imageHeight * pixelRatio).toBeCloseTo(626)
      expect(size.stageWidth).toBeCloseTo(size.imageWidth + layout.border * 2)
      expect(size.stageHeight).toBeCloseTo(size.imageHeight + layout.border * 2)
    }
  })
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
        expect(size.stageHeight).toBeLessThanOrEqual(290 - layout.padding * 2 - layout.gap - (expanded ? layout.expandedHeight : layout.collapsedHeight))
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


describe('贴图圆角与自动阴影', () => {
  it('阴影随实际尺寸、透明度和焦点调整，关闭时完全去除', () => {
    const appearance = defaultPinAppearance()
    const small = pinAppearanceStyle(appearance, 40, 30)
    const large = pinAppearanceStyle(appearance, 800, 600)
    expect(small.boxShadow).not.toBe(large.boxShadow)
    expect(pinAppearanceStyle(appearance, 800, 600, 1, true).boxShadow).not.toBe(large.boxShadow)
    expect(pinAppearanceStyle(appearance, 800, 600, 0.3).boxShadow).not.toBe(large.boxShadow)
    expect(pinAppearanceStyle({ ...appearance, shadow: 'none' }, 800, 600).boxShadow).toBe('none')
  })
  it('圆角遵循设定并受显示短边限制，原始像素尺寸不参与缩放重复计算', () => {
    expect(pinAppearanceStyle({ ...defaultPinAppearance(), cornerRadius: 24 }, 40, 30).borderRadius).toBe('15px')
    expect(pinAppearanceStyle({ ...defaultPinAppearance(), shadow: 'soft', cornerRadius: 0 }, 400, 300).borderRadius).toBe('0px')
    expect(validatePinAppearance(defaultPinAppearance())).toBeNull()
    expect(validatePinAppearance({ ...defaultPinAppearance(), cornerRadius: 25 })).not.toBeNull()
  })
  it('自定义边框颜色用于所有阴影及焦点状态，拒绝无效颜色', () => {
    const appearance = { ...defaultPinAppearance(), borderColor: '#12aBcD' }
    for (const shadow of ['auto', 'soft', 'none'] as const) {
      for (const focused of [true, false]) expect(pinAppearanceStyle({ ...appearance, shadow }, 400, 300, 1, focused).borderColor).toBe('#12aBcD')
    }
    expect(validatePinAppearance(appearance)).toBeNull()
    for (const borderColor of ['', '#abc', '#12345678', 'red', '#gggggg']) expect(validatePinAppearance({ ...appearance, borderColor })).not.toBeNull()
  })

})
