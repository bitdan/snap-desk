import { afterEach, expect, it, vi } from 'vitest'
import { prepareRaster } from '../../src/features/annotations/annotation-raster'
import { shapeBounds } from '../../src/features/annotations/annotations'

afterEach(() => vi.unstubAllGlobals())
it.each([
  { text: Array(20).fill('多行文字').join('\n'), fontSize: 24, width: 600, height: 1000, scaled: false },
  { text: Array(5).fill('字'.repeat(80)).join('\n'), fontSize: 128, width: 2048, height: 2048, scaled: true },
])('文字按实际选区排版且 PNG 满足原生限额：$fontSize px', async ({ text, fontSize, width, height, scaled }) => {
  const context = { font: '', measureText: (value: string) => ({ width: [...value].length * 20 }), fillText: vi.fn(), scale: vi.fn(), fillStyle: '', textBaseline: '' }
  let encoded = { width: 0, height: 0 }
  const canvas = { width: 0, height: 0, getContext: () => context, toDataURL: () => {
    encoded = { width: canvas.width, height: canvas.height }; return 'data:image/png;base64,glyph'
  } }
  vi.stubGlobal('document', { fonts: { ready: Promise.resolve() }, createElement: () => canvas })
  const shape = await prepareRaster({ kind: 'text', start: { x: 50, y: 60 }, end: { x: 50, y: 60 }, color: '#ef4444', width: 3, text, fontSize }, '', { x: 50, y: 60, width, height })
  const box = shapeBounds(shape)
  expect(box.height).toBeGreaterThan(512)
  expect(box.height).toBeLessThanOrEqual(height)
  expect(box.x).toBe(50); expect(box.y).toBe(60)
  expect(encoded.width).toBeLessThanOrEqual(2048)
  expect(encoded.height).toBeLessThanOrEqual(2048)
  expect(encoded.width * encoded.height).toBeLessThanOrEqual(1_048_576)
  expect(context.scale.mock.calls.length > 0).toBe(scaled)
  expect(canvas.width).toBe(0); expect(canvas.height).toBe(0)
})
