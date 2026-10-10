import { describe, expect, it } from 'vitest'
import { move, normalize, resize, toPixels } from '../src/lib/selection'

describe('screen selection coordinates', () => {
  it('accepts reverse drags', () => {
    expect(normalize({ x: 90, y: 80 }, { x: 10, y: 20 })).toEqual({ x: 10, y: 20, width: 80, height: 60 })
  })
  it('clamps movement at all screen edges without changing size', () => {
    const rect = { x: 20, y: 10, width: 40, height: 30 }
    expect(move(rect, { x: -100, y: -100 }, { width: 100, height: 100 })).toEqual({ ...rect, x: 0, y: 0 })
    expect(move(rect, { x: 100, y: 100 }, { width: 100, height: 100 })).toEqual({ ...rect, x: 60, y: 70 })
  })
  it('allows a resize handle to cross the opposite corner', () => {
    expect(resize({ x: 10, y: 20, width: 30, height: 40 }, 'nw', { x: 60, y: 80 })).toEqual({ x: 40, y: 60, width: 20, height: 20 })
  })
  it('maps 125% scaling with independently rounded edges', () => {
    expect(toPixels({ x: 1, y: 3, width: 10, height: 10 }, { width: 1536, height: 864 }, { width: 1920, height: 1080 })).toEqual({ x: 1, y: 3, width: 13, height: 14 })
  })
  it('preserves full image at 200% and clips rounding at the image boundary', () => {
    expect(toPixels({ x: 0, y: 0, width: 1440, height: 900 }, { width: 1440, height: 900 }, { width: 2880, height: 1800 })).toEqual({ x: 0, y: 0, width: 2880, height: 1800 })
    expect(toPixels({ x: 99.5, y: 99.5, width: 1, height: 1 }, { width: 100, height: 100 }, { width: 200, height: 200 })).toEqual({ x: 199, y: 199, width: 1, height: 1 })
  })
  it('resizes an edge without changing the other axis', () => {
    expect(resize({ x: 10, y: 20, width: 30, height: 40 }, 'e', { x: 80, y: 0 })).toEqual({ x: 10, y: 20, width: 70, height: 40 })
  })
})
