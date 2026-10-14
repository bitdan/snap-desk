import { describe, expect, it } from 'vitest'
import { filterPins, pinGroups, normalizePinGroup } from '../src/lib/pin-content'
import type { PinSummary } from '../src/lib/api'

describe('pin groups', () => {
  const pins: PinSummary[] = [
    { label: 'pin-1', width: 20, height: 20, visible: true, group: '' },
    { label: 'pin-2', width: 20, height: 20, visible: false, group: '参考' },
    { label: 'pin-3', width: 20, height: 20, visible: true, group: '参考' },
  ]
  it('distinguishes all pins from ungrouped pins and includes hidden group members', () => {
    expect(filterPins(pins, null)).toHaveLength(3)
    expect(filterPins(pins, '')).toEqual([pins[0]])
    expect(filterPins(pins, '参考')).toEqual(pins.slice(1))
    expect(filterPins(pins, '不存在')).toEqual([])
    expect(pinGroups(pins)).toEqual(['参考'])
    expect(pinGroups([])).toEqual([])
  })
  it('trims names and allows moving back to ungrouped without accepting control characters', () => {
    expect(normalizePinGroup(' 参考 ')).toBe('参考')
    expect(normalizePinGroup(' ')).toBe('')
    expect(normalizePinGroup('😀'.repeat(32))).toHaveLength(64)
    for (const name of ['字'.repeat(33), 'a\nb', 'a\u0085b']) expect(() => normalizePinGroup(name)).toThrow()
  })
})
