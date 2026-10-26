import { describe, expect, it } from 'vitest'
import { rowAt, rowOffsets, visibleRows } from '../src/lib/virtual-list'

describe('可变高度历史列表', () => {
  it('limits rendered rows when browsing thousands of records and preserves total height', () => {
    const offsets = rowOffsets(Array.from({ length: 5000 }, (_, i) => i % 2 ? 140 : 78))
    for (const top of [0, 10000, offsets.at(-1)! - 500]) {
      const range = visibleRows(offsets, top, 500)
      expect(range.end - range.start).toBeLessThan(20)
      expect(range.before + offsets[range.end] - offsets[range.start] + range.after).toBe(offsets.at(-1))
      expect(range.start).toBeLessThanOrEqual(rowAt(offsets, top))
      expect(range.end).toBeGreaterThan(rowAt(offsets, top + 499))
    }
  })
  it('locates exact row boundaries for keyboard navigation after measurements change', () => {
    expect(rowAt(rowOffsets([78, 140, 78]), 78)).toBe(1)
    expect(rowAt(rowOffsets([120, 140, 78]), 78)).toBe(0)
    expect(rowAt(rowOffsets([120, 140, 78]), 10000)).toBe(2)
    expect(visibleRows([0], 100, 500)).toEqual({ start: 0, end: 0, before: 0, after: 0 })
  })
})
