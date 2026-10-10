import { describe, expect, it } from 'vitest'
import { colorLabels, nudgeColorPoint } from '../src/lib/color-picker'
import { magnifierSample } from '../src/lib/magnifier'

describe('screen color picker', () => {
  it('formats leading zeros, mixed channels and white consistently with native copy', () => {
    expect(colorLabels(0, 128, 255)).toEqual({ hex: '#0080FF', rgb: 'rgb(0, 128, 255)' })
    expect(colorLabels(1, 2, 3).hex).toBe('#010203')
    expect(colorLabels(255, 255, 255).hex).toBe('#FFFFFF')
  })
  it('nudges exact source pixels repeatedly at fractional, nonuniform DPI', () => {
    const viewport = { width: 1536, height: 777 }, image = { width: 1920, height: 1080 }
    let point = { x: 100, y: 100 }
    const initial = magnifierSample(point, viewport, image).pixel
    for (let i = 0; i < 100; i++) point = nudgeColorPoint(point, { x: 1, y: 1 }, viewport, image)
    expect(magnifierSample(point, viewport, image).pixel).toEqual({ x: initial.x + 100, y: initial.y + 100 })
    for (let i = 0; i < 100; i++) point = nudgeColorPoint(point, { x: -1, y: -1 }, viewport, image)
    expect(magnifierSample(point, viewport, image).pixel).toEqual(initial)
  })
  it('clamps accelerated movement to the first and last valid pixel', () => {
    const viewport = { width: 80, height: 80 }, image = { width: 100, height: 100 }
    const low = nudgeColorPoint({ x: 0, y: 0 }, { x: -10, y: -10 }, viewport, image)
    const high = nudgeColorPoint({ x: 80, y: 80 }, { x: 10, y: 10 }, viewport, image)
    expect(magnifierSample(low, viewport, image).pixel).toEqual({ x: 0, y: 0 })
    expect(magnifierSample(high, viewport, image).pixel).toEqual({ x: 99, y: 99 })
  })
})
