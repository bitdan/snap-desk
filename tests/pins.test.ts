import { describe, expect, it } from 'vitest'
import { canEditPin, minimumZoom, stepZoom } from '../src/lib/pins'

describe('pin display controls', () => {
  it('keeps small and narrow images visible after repeated shrinking', () => {
    for (const [width, height, minimum] of [[200, 100, 0.48], [100, 200, 0.48], [20, 300, 1], [800, 600, 0.1]]) {
      expect(minimumZoom(width, height)).toBe(minimum)
      let zoom = 1
      for (let i = 0; i < 100; i++) zoom = stepZoom(zoom, -1, minimum)
      expect(zoom).toBe(minimum)
      expect(stepZoom(zoom, 1, minimum)).toBeGreaterThan(minimum)
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
