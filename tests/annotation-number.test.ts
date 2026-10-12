import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useAnnotations } from '../src/lib/use-annotations'
import { nextNumber, isRaster } from '../src/lib/annotations'
import { prepareRaster } from '../src/lib/annotation-raster'

vi.mock('../src/lib/annotation-raster', () => ({ prepareRaster: vi.fn() }))
const bounds = () => ({ x: 20, y: 30, width: 200, height: 100 })
function create(persist?: Parameters<typeof useAnnotations>[3]) {
  const editor = useAnnotations(bounds, () => '', () => false, persist)
  editor.setTool('number')
  return editor
}
async function place(editor: ReturnType<typeof create>) {
  editor.begin({ x: 80, y: 70 }, 2)
  await editor.finish()
}
beforeEach(() => vi.mocked(prepareRaster).mockReset().mockImplementation(async shape => ({ ...shape, raster: 'number-png' })))

describe('number annotations shared by capture and pins', () => {
  it('places sequential numbers with one click and reuses the undone last number', async () => {
    const editor = create()
    await place(editor); await place(editor)
    expect(editor.history.present.map(s => s.text)).toEqual(['1', '2'])
    expect(editor.history.present.every(isRaster)).toBe(true)
    await editor.travel('undo'); await place(editor)
    expect(editor.history.present.map(s => s.text)).toEqual(['1', '2'])
    expect(editor.history.future).toEqual([])
  })
  it('keeps markers inside an offset crop and ignores drag during placement', async () => {
    const editor = create()
    editor.begin({ x: 220, y: 130 }, 2); editor.drag({ x: 20, y: 30 })
    await editor.finish()
    expect(editor.history.present[0]).toMatchObject({ start: { x: 184, y: 94 }, end: { x: 220, y: 130 } })
  })
  it('preserves numbering across imported pins and middle deletions, resets on clear', async () => {
    const editor = create()
    await place(editor)
    const shape = editor.history.present[0]
    editor.reset([{ ...shape, text: '2' }, { ...shape, text: '7' }])
    editor.select(0); await editor.remove()
    expect(nextNumber(editor.history.present)).toBe(8)
    await place(editor)
    expect(editor.history.present.map(s => s.text)).toEqual(['7', '8'])
    await editor.clear(); await place(editor)
    expect(editor.history.present[0].text).toBe('1')
  })
  it('does not consume a number on cancellation or failed persistence', async () => {
    const editor = create(vi.fn().mockRejectedValue(new Error('save failed')))
    editor.begin({ x: 80, y: 70 }, 2); editor.abort()
    await place(editor)
    expect(editor.history.present).toEqual([])
    expect(nextNumber(editor.history.present)).toBe(1)
    expect(editor.error).toContain('save failed')
  })
  it('supports move, resize, recolor, and opacity without losing the label', async () => {
    const editor = create()
    await place(editor); editor.setTool('edit')
    editor.begin({ x: 80, y: 70 }, 2); editor.drag({ x: 90, y: 80 }); await editor.finish()
    editor.begin({ x: 108, y: 98 }, 2, 'se'); editor.drag({ x: 118, y: 108 }); await editor.finish()
    expect(editor.history.present[0]).toMatchObject({ text: '1', start: { x: 72, y: 62 }, end: { x: 118, y: 108 } })
    await editor.style({ color: '#00ff00' })
    const calls = vi.mocked(prepareRaster).mock.calls.length
    await editor.style({ opacity: .35 })
    expect(prepareRaster).toHaveBeenCalledTimes(calls)
    expect(editor.history.present[0]).toMatchObject({ text: '1', color: '#00ff00', opacity: .35 })
  })
  it('rejects tiny selections and sequence overflow', async () => {
    const tiny = useAnnotations(() => ({ x: 0, y: 0, width: 10, height: 10 }), () => '', () => false)
    tiny.setTool('number'); tiny.begin({ x: 5, y: 5 }, 2)
    expect(tiny.error).toContain('选区太小')
    const editor = create(); await place(editor)
    editor.reset([{ ...editor.history.present[0], text: '999' }])
    editor.begin({ x: 80, y: 70 }, 2)
    expect(editor.error).toContain('999')
    expect(editor.interacting).toBe(false)
  })
})
