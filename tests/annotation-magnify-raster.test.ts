import { afterEach, expect, it, vi } from 'vitest'
import { prepareRaster } from '../src/lib/annotation-raster'
import type { Annotation } from '../src/lib/annotations'

afterEach(() => vi.unstubAllGlobals())
it('samples the correct source pixels, bounds the raster, and keeps geometry and opacity', async () => {
  const context = { drawImage: vi.fn(), strokeRect: vi.fn(), imageSmoothingEnabled: false, lineWidth: 0, strokeStyle: '' }
  const canvas = { width: 0, height: 0, getContext: () => context, toDataURL: () => 'data:image/png;base64,result' }
  vi.stubGlobal('document', { createElement: () => canvas })
  const removeAttribute = vi.fn()
  vi.stubGlobal('Image', class { src = ''; decode = vi.fn().mockResolvedValue(undefined); removeAttribute = removeAttribute })
  const shape: Annotation = { kind: 'magnify', start: { x: 2100, y: 1100 }, end: { x: 100, y: 100 }, width: 4, color: '#123456', opacity: .7, magnification: 4 }
  const result = await prepareRaster(shape, 'source', { x: 0, y: 0, width: 3000, height: 2000 })
  expect(canvas.width).toBe(0); expect(canvas.height).toBe(0)
  expect(removeAttribute).toHaveBeenCalledWith('src')
  expect(context.drawImage).toHaveBeenCalledWith(expect.anything(), 850, 475, 500, 250, 0, 0, 1024, 512)
  expect(context.strokeStyle).toBe('#123456')
  expect(context.lineWidth).toBeCloseTo(2.048)
  expect(result).toMatchObject({ start: { x: 100, y: 100 }, end: { x: 2100, y: 1100 }, opacity: .7, raster: 'result', magnification: 4 })
})

const crop = { x: 0, y: 0, width: 200, height: 100 }
const shape: Annotation = { kind: 'mosaic', start: { x: 0, y: 0 }, end: { x: 100, y: 80 }, width: 8, color: '#123456' }

it.each(['mosaic', 'blur'] as const)('releases every temporary %s canvas and decoded image', async kind => {
  const canvases: { width: number; height: number }[] = []
  const context = { drawImage: vi.fn(), imageSmoothingEnabled: true, filter: '' }
  vi.stubGlobal('document', { createElement: () => {
    const canvas = { width: 0, height: 0, getContext: () => context, toDataURL: () => 'data:image/png;base64,result' }
    canvases.push(canvas); return canvas
  } })
  const removeAttribute = vi.fn()
  vi.stubGlobal('Image', class { src = ''; decode = vi.fn().mockResolvedValue(undefined); removeAttribute = removeAttribute })
  expect(await prepareRaster({ ...shape, kind }, 'source', crop)).toMatchObject({ raster: 'result' })
  expect(canvases).toHaveLength(2)
  expect(canvases.every(canvas => canvas.width === 0 && canvas.height === 0)).toBe(true)
  expect(removeAttribute).toHaveBeenCalledWith('src')
})

it.each(['decode', 'encode', 'oversize'] as const)('releases resources after %s failure', async failure => {
  const canvas = { width: 0, height: 0, getContext: () => ({ drawImage: vi.fn(), strokeRect: vi.fn() }), toDataURL: () => {
    if (failure === 'encode') throw new Error('encoding failed')
    return `data:image/png;base64,${'x'.repeat(2_000_001)}`
  } }
  vi.stubGlobal('document', { createElement: () => canvas })
  const removeAttribute = vi.fn()
  vi.stubGlobal('Image', class {
    src = ''; removeAttribute = removeAttribute
    decode = () => failure === 'decode' ? Promise.reject(new Error('decode failed')) : Promise.resolve()
  })
  await expect(prepareRaster({ ...shape, kind: 'magnify' }, 'source', crop)).rejects.toThrow()
  expect(canvas.width).toBe(0); expect(canvas.height).toBe(0)
  expect(removeAttribute).toHaveBeenCalledWith('src')
})
