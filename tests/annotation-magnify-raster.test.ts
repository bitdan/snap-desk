import { afterEach, expect, it, vi } from 'vitest'
import { prepareRaster } from '../src/lib/annotation-raster'
import type { Annotation } from '../src/lib/annotations'

afterEach(() => vi.unstubAllGlobals())
it('samples the correct source pixels, bounds the raster, and keeps geometry and opacity', async () => {
  const context = { drawImage: vi.fn(), strokeRect: vi.fn(), imageSmoothingEnabled: false, lineWidth: 0, strokeStyle: '' }
  const canvas = { width: 0, height: 0, getContext: () => context, toDataURL: () => 'data:image/png;base64,result' }
  vi.stubGlobal('document', { createElement: () => canvas })
  vi.stubGlobal('Image', class { src = ''; decode = vi.fn().mockResolvedValue(undefined) })
  const shape: Annotation = { kind: 'magnify', start: { x: 2100, y: 1100 }, end: { x: 100, y: 100 }, width: 4, color: '#123456', opacity: .7, magnification: 4 }
  const result = await prepareRaster(shape, 'source', { x: 0, y: 0, width: 3000, height: 2000 })
  expect(canvas.width).toBe(1024); expect(canvas.height).toBe(512)
  expect(context.drawImage).toHaveBeenCalledWith(expect.anything(), 850, 475, 500, 250, 0, 0, 1024, 512)
  expect(context.strokeStyle).toBe('#123456')
  expect(context.lineWidth).toBeCloseTo(2.048)
  expect(result).toMatchObject({ start: { x: 100, y: 100 }, end: { x: 2100, y: 1100 }, opacity: .7, raster: 'result', magnification: 4 })
})
