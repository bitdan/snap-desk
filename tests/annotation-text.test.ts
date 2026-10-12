import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useAnnotations } from '../src/lib/use-annotations'
import { prepareRaster } from '../src/lib/annotation-raster'

vi.mock('../src/lib/annotation-raster', () => ({ prepareRaster: vi.fn() }))
const bounds = () => ({ x: 20, y: 30, width: 600, height: 400 })
function editorFor(persist?: Parameters<typeof useAnnotations>[3]) {
  const editor = useAnnotations(bounds, () => 'image', () => false, persist)
  editor.setTool('text')
  return editor
}
beforeEach(() => {
  vi.mocked(prepareRaster).mockReset().mockImplementation(async shape => ({ ...shape, end: { x: shape.start.x + 80, y: shape.start.y + 40 }, raster: 'glyph' }))
})

describe('place a text input before typing', () => {
  it('opens an empty draft at the clicked position without committing on pointerup', async () => {
    const editor = editorFor()
    editor.text = 'previous text'
    editor.begin({ x: 80, y: 90 }, 2)
    await editor.finish()
    expect(editor.textEditing).toBe(true)
    expect(editor.draft).toMatchObject({ text: '', start: { x: 80, y: 90 } })
    expect(editor.history.present).toEqual([])
    expect(prepareRaster).not.toHaveBeenCalled()
  })
  it('commits multiline text only on completion and supports undo/redo', async () => {
    const persist = vi.fn().mockResolvedValue(undefined), editor = editorFor(persist)
    editor.begin({ x: 80, y: 90 }, 2)
    editor.draft!.text = '中文\n第二行'
    await editor.finishText()
    expect(persist).toHaveBeenCalledTimes(1)
    expect(editor.history.present[0]).toMatchObject({ text: '中文\n第二行', raster: 'glyph' })
    expect(prepareRaster).toHaveBeenCalledWith(expect.anything(), 'image', { x: 80, y: 90, width: 180, height: 180 })
    expect(editor.interacting).toBe(false)
    await editor.travel('undo'); expect(editor.history.present).toEqual([])
    await editor.travel('redo'); expect(editor.history.present[0].text).toBe('中文\n第二行')
  })
  it('cancels empty or abandoned drafts without adding history', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = ' \n '
    await editor.finishText()
    expect(editor.textEditing).toBe(false)
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '取消'
    editor.abort()
    expect(editor.history.past).toEqual([])
    expect(editor.history.present).toEqual([])
    expect(prepareRaster).not.toHaveBeenCalled()
  })
  it('keeps the input inside offset bounds near screen edges', () => {
    const editor = editorFor()
    editor.begin({ x: 620, y: 430 }, 2)
    expect(editor.textBox()).toEqual({ x: 500, y: 394, width: 120, height: 36 })
  })
  it('retains typed text after raster or native persistence failures', async () => {
    const persist = vi.fn().mockRejectedValue(new Error('native failure')), editor = editorFor(persist)
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '保留输入'
    vi.mocked(prepareRaster).mockRejectedValueOnce(new Error('layout failure'))
    await editor.finishText()
    expect(editor.error).toContain('layout failure')
    expect(persist).not.toHaveBeenCalled()
    await editor.finishText()
    expect(editor.error).toContain('native failure')
    expect(editor.draft!.text).toBe('保留输入')
    expect(editor.textEditing).toBe(true)
    expect(editor.pending).toBe(false)
    expect(editor.history.present).toEqual([])
    persist.mockResolvedValue(undefined)
    await editor.finishText()
    expect(editor.history.present).toHaveLength(1)
  })
  it('does not commit an old draft after the image resets during rasterization', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '旧文字'
    const old = { ...editor.draft! }
    let release!: (shape: typeof old) => void
    vi.mocked(prepareRaster).mockImplementationOnce(() => new Promise(resolve => { release = resolve }))
    const task = editor.finishText()
    editor.reset(); release(old); await task
    expect(editor.history.present).toEqual([])
    expect(editor.textEditing).toBe(false)
  })
})
