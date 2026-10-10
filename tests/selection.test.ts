import { describe, expect, it } from 'vitest'
import { move, normalize, resize, toPixels, nudgeSelection } from '../src/lib/selection'

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
  it('moves by exact physical pixels at fractional and independent axis scales', () => {
    const viewport = { width: 1536, height: 777 }
    const image = { width: 1920, height: 1080 }
    let rect = { x: 8, y: 14, width: 40, height: 30 }
    const original = toPixels(rect, viewport, image)
    for (let i = 0; i < 100; i++) rect = nudgeSelection(rect, { x: 1, y: 1 }, viewport, image)
    expect(toPixels(rect, viewport, image)).toEqual({ ...original, x: original.x + 100, y: original.y + 100 })
    for (let i = 0; i < 100; i++) rect = nudgeSelection(rect, { x: -1, y: -1 }, viewport, image)
    expect(toPixels(rect, viewport, image)).toEqual(original)
  })
  it('clamps accelerated keyboard movement while preserving the pixel size', () => {
    const size = { width: 100, height: 80 }
    const rect = { x: 5, y: 6, width: 90, height: 70 }
    expect(nudgeSelection(rect, { x: 10, y: 10 }, size, size)).toEqual({ ...rect, x: 10, y: 10 })
    expect(nudgeSelection(rect, { x: -10, y: -10 }, size, size)).toEqual({ ...rect, x: 0, y: 0 })
    expect(nudgeSelection({ x: 0, y: 0, ...size }, { x: 10, y: -10 }, size, size)).toEqual({ x: 0, y: 0, ...size })
  })
  it('keeps the top-left corner fixed and constrains keyboard resizing to the image', () => {
    const viewport = { width: 80, height: 80 }
    const image = { width: 100, height: 100 }
    const rect = { x: 8, y: 16, width: 40, height: 32 }
    const shrink = nudgeSelection(rect, { x: -100, y: -100 }, viewport, image, true)
    expect(toPixels(shrink, viewport, image)).toEqual({ x: 10, y: 20, width: 1, height: 1 })
    const grow = nudgeSelection(rect, { x: 100, y: 100 }, viewport, image, true)
    expect(toPixels(grow, viewport, image)).toEqual({ x: 10, y: 20, width: 90, height: 80 })
    expect(toPixels(nudgeSelection(rect, { x: 1, y: 0 }, viewport, image, true), viewport, image)).toEqual({ x: 10, y: 20, width: 51, height: 40 })
  })
})
