import { describe, expect, it, vi, beforeEach } from 'vitest'
import { colorPinRequest, textPinRequest, filterPins, pinGroups, normalizePinGroup } from '../src/lib/pin-content'
import { prepareRaster } from '../src/lib/annotation-raster'
import type { PinSummary } from '../src/lib/api'

vi.mock('../src/lib/annotation-raster', () => ({ prepareRaster: vi.fn(async shape => ({ ...shape, raster: 'glyph', end: { x: 120, y: 60 } })) }))
beforeEach(() => vi.clearAllMocks())

describe('new pin inputs', () => {
  it('normalizes HEX and validates integer image dimensions', () => {
    expect(colorPinRequest(' #aB1234 ', 16, 2048)).toEqual({ color: '#AB1234', width: 16, height: 2048 })
    for (const width of [NaN, Infinity, 0, 15, 2049, 16.5]) expect(() => colorPinRequest('#123456', width, 20)).toThrow()
    for (const color of ['red', '#123', '#12345g']) expect(() => colorPinRequest(color, 20, 20)).toThrow()
  })
  it('preserves multiline content and editable glyph metadata with padding', async () => {
    const result = await textPinRequest('中文\nhello', '#000000', '#fff4b8', 24)
    expect(result.background).toBe('#FFF4B8')
    expect(result.annotation).toMatchObject({ kind: 'text', text: '中文\nhello', fontSize: 24, start: { x: 16, y: 16 }, raster: 'glyph' })
    expect(prepareRaster).toHaveBeenCalledWith(expect.anything(), '', { x: 16, y: 16, width: 608, height: 480 })
  })
  it('rejects empty, oversized and invalid inputs before rasterizing', async () => {
    for (const text of ['  \n ', '字'.repeat(501)]) await expect(textPinRequest(text, '#000000', '#ffffff', 24)).rejects.toThrow()
    await expect(textPinRequest('text', '#000000', '#ffffff', 7)).rejects.toThrow()
    await expect(textPinRequest('text', '#bad', '#ffffff', 24)).rejects.toThrow()
    expect(prepareRaster).not.toHaveBeenCalled()
    await textPinRequest('😀'.repeat(500), '#000000', '#ffffff', 24)
    expect(prepareRaster).toHaveBeenCalledOnce()
  })
  it('propagates layout failure without silently truncating the text', async () => {
    vi.mocked(prepareRaster).mockRejectedValueOnce(new Error('文字超出区域'))
    await expect(textPinRequest('长文本', '#000000', '#ffffff', 128)).rejects.toThrow('文字超出区域')
  })
})

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
