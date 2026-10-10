import { describe, expect, it } from 'vitest'
import { magnifierPosition, magnifierSample, MAGNIFIER_ZOOM } from '../src/lib/magnifier'

describe('capture pixel magnifier', () => {
  it('maps fractional DPI and independent axis scales to original pixels', () => {
    const sample = magnifierSample({ x: 101, y: 101 }, { width: 1536, height: 720 }, { width: 1920, height: 1080 })
    expect(sample).toEqual({ pixel: { x: 126, y: 151 }, x: 119, y: 144, width: 15, height: 15, dx: 0, dy: 0 })
  })

  it('keeps the sampled pixel centered at all four image edges', () => {
    const size = { width: 100, height: 80 }
    for (const x of [0, 100]) for (const y of [0, 80]) {
      const s = magnifierSample({ x, y }, size, size)
      expect(s.pixel).toEqual({ x: Math.min(x, 99), y: Math.min(y, 79) })
      expect(s.width).toBe(8)
      expect(s.height).toBe(8)
      expect(s.dx + (s.pixel.x - s.x) * MAGNIFIER_ZOOM).toBe(56)
      expect(s.dy + (s.pixel.y - s.y) * MAGNIFIER_ZOOM).toBe(56)
      expect(s.x + s.width).toBeLessThanOrEqual(size.width)
      expect(s.y + s.height).toBeLessThanOrEqual(size.height)
    }
  })

  it('supports images smaller than the sampling area and out-of-bounds pointers', () => {
    expect(magnifierSample({ x: -10, y: 500 }, { width: 100, height: 100 }, { width: 1, height: 1 }))
      .toEqual({ pixel: { x: 0, y: 0 }, x: 0, y: 0, width: 1, height: 1, dx: 56, dy: 56 })
  })

  it('places the panel away from the pointer and flips at screen edges', () => {
    const viewport = { width: 800, height: 600 }, panel = { width: 122, height: 150 }
    expect(magnifierPosition({ x: 100, y: 100 }, viewport, panel)).toEqual({ x: 124, y: 124 })
    expect(magnifierPosition({ x: 790, y: 590 }, viewport, panel)).toEqual({ x: 644, y: 416 })
    expect(magnifierPosition({ x: 40, y: 40 }, { width: 80, height: 80 }, panel)).toEqual({ x: 0, y: 0 })
  })
})
