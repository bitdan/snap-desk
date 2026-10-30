import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { prepareRaster } from '../../src/features/annotations/annotation-raster'

vi.mock('../../src/features/annotations/annotation-raster', () => ({ prepareRaster: vi.fn() }))
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
  it('opens text at the arrow midpoint and saves both layers in one undoable operation', async () => {
    const persist = vi.fn().mockResolvedValue(undefined), editor = editorFor(persist)
    editor.setTool('arrow-text')
    editor.begin({ x: 100, y: 100 }, 2); editor.drag({ x: 400, y: 200 }); await editor.finish()
    expect(editor.textEditing).toBe(true)
    expect(editor.textBox()).toEqual({ x: 160, y: 132, width: 180, height: 298 })
    expect(editor.arrowDraft).toMatchObject({ kind: 'arrow', start: { x: 100, y: 100 }, end: { x: 400, y: 200 } })
    expect(editor.draft).toMatchObject({ kind: 'text', text: '' })
    expect(persist).not.toHaveBeenCalled()
    editor.draft!.text = '箭头说明'; await editor.finishText()
    expect(editor.history.present.map(s => s.kind)).toEqual(['arrow', 'text'])
    expect(editor.history.present[1].start.x).toBe(210)
    expect(persist).toHaveBeenCalledTimes(1)
    await editor.travel('undo'); expect(editor.history.present).toEqual([])
    await editor.travel('redo'); expect(editor.history.present).toHaveLength(2)
  })
  it('keeps a plain arrow when its label is empty and cancels the whole draft on abort', async () => {
    const editor = editorFor()
    editor.setTool('arrow-text')
    editor.begin({ x: 100, y: 100 }, 2); editor.drag({ x: 400, y: 100 }); await editor.finish()
    await editor.finishText()
    expect(editor.history.present.map(s => s.kind)).toEqual(['arrow'])
    expect(prepareRaster).not.toHaveBeenCalled()
    editor.begin({ x: 100, y: 100 }, 2); editor.drag({ x: 400, y: 100 }); await editor.finish()
    editor.abort()
    expect(editor.arrowDraft).toBeNull()
    expect(editor.history.present).toHaveLength(1)
  })
  it('retains both arrow and text for retry after persistence fails', async () => {
    const persist = vi.fn().mockRejectedValueOnce(new Error('save failed')).mockResolvedValue(undefined)
    const editor = editorFor(persist)
    editor.setTool('arrow-text')
    editor.begin({ x: 620, y: 430 }, 2); editor.drag({ x: 600, y: 420 }); await editor.finish()
    const box = editor.textBox()
    expect(box.x + box.width).toBeLessThanOrEqual(620)
    expect(box.y + box.height).toBeLessThanOrEqual(430)
    editor.draft!.text = '重试'; await editor.finishText()
    expect(editor.textEditing).toBe(true)
    expect(editor.arrowDraft?.kind).toBe('arrow')
    expect(editor.history.present).toEqual([])
    await editor.finishText()
    expect(editor.history.present).toHaveLength(2)
    expect(editor.arrowDraft).toBeNull()
  })
  it('reopens existing text with the text tool and replaces it in one undoable edit', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '原文'
    await editor.finishText()
    const original = editor.history.present[0]
    editor.editText(0)
    expect(editor.textEditing).toBe(true)
    expect(editor.textAutoHeight).toBe(true)
    editor.draft!.text = '修改后'
    expect(original.text).toBe('原文')
    await editor.finishText()
    expect(editor.history.present).toHaveLength(1)
    expect(editor.history.present[0].text).toBe('修改后')
    await editor.travel('undo')
    expect(editor.history.present).toEqual([original])
    await editor.travel('redo')
    expect(editor.history.present[0].text).toBe('修改后')
  })
  it('single click selects text, explicit editing clones it, and dragging moves it', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '文字'
    await editor.finishText()
    editor.setTool('edit')
    editor.begin({ x: 90, y: 100 }, 2); await editor.finish()
    expect(editor.textEditing).toBe(false)
    expect(editor.selected).toBe(0)
    expect(editor.history.past).toHaveLength(1)
    editor.editText(0)
    expect(editor.textEditing).toBe(true)
    editor.draft!.text = '取消修改'; editor.abort()
    expect(editor.history.present[0].text).toBe('文字')
    editor.begin({ x: 90, y: 100 }, 2); editor.drag({ x: 110, y: 120 }); await editor.finish()
    expect(editor.textEditing).toBe(false)
    expect(editor.history.present[0].start).toEqual({ x: 100, y: 110 })
  })
  it('resizes input width and height independently within crop bounds before committing', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '调整宽高'
    editor.beginTextResize({ x: 260, y: 108 }, 'e', 36)
    editor.drag({ x: 400, y: 200 }); await editor.finish()
    expect(editor.textAutoHeight).toBe(false)
    expect(editor.textBox()).toEqual({ x: 80, y: 90, width: 320, height: 36 })
    editor.beginTextResize({ x: 240, y: 126 }, 's', 36)
    editor.drag({ x: 300, y: 900 }); await editor.finish()
    expect(editor.textBox()).toEqual({ x: 80, y: 90, width: 320, height: 340 })
    expect(editor.history.present).toEqual([])
    await editor.finishText()
    expect(prepareRaster).toHaveBeenLastCalledWith(expect.anything(), 'image', { x: 80, y: 90, width: 320, height: 340 })
    editor.begin({ x: 90, y: 100 }, 2)
    expect(editor.textAutoHeight).toBe(true)
  })
  it('keeps the original layer and edited draft when saving a second edit fails', async () => {
    const persist = vi.fn().mockResolvedValue(undefined), editor = editorFor(persist)
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '原文'; await editor.finishText()
    editor.editText(0); editor.draft!.text = '新内容'
    persist.mockRejectedValueOnce(new Error('save failed'))
    await editor.finishText()
    expect(editor.history.present[0].text).toBe('原文')
    expect(editor.draft!.text).toBe('新内容')
    expect(editor.textEditing).toBe(true)
    await editor.finishText()
    expect(editor.history.present).toHaveLength(1)
    expect(editor.history.present[0].text).toBe('新内容')
  })
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
    expect(prepareRaster).toHaveBeenCalledWith(expect.anything(), 'image', { x: 80, y: 90, width: 180, height: 340 })
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
  it('lets the user adjust font size after a layout failure without losing text or locking the toolbar', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '需要缩小字号'
    vi.mocked(prepareRaster).mockRejectedValueOnce(new Error('文字超出区域'))
    await editor.finishText()
    expect(editor.textEditing).toBe(true)
    await editor.style({ fontSize: 16, color: '#123456' })
    expect(editor.draft).toMatchObject({ text: '需要缩小字号', fontSize: 16, color: '#123456' })
    expect(editor.error).toBe('')
    await editor.finishText()
    expect(editor.textEditing).toBe(false)
    expect(editor.history.present[0]).toMatchObject({ text: '需要缩小字号', fontSize: 16 })
  })
  it('Esc can cancel pending text layout and late results cannot restore the abandoned input', async () => {
    const editor = editorFor()
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '等待排版'
    const old = { ...editor.draft! }
    let release!: (shape: typeof old) => void
    vi.mocked(prepareRaster).mockImplementationOnce(() => new Promise(resolve => { release = resolve }))
    const task = editor.finishText(); expect(editor.pending).toBe(true)
    editor.cancelText()
    expect(editor.textEditing).toBe(false); expect(editor.pending).toBe(false)
    editor.begin({ x: 80, y: 90 }, 2); editor.draft!.text = '新输入'
    release(old); await task
    expect(editor.history.present).toEqual([])
    expect(editor.draft!.text).toBe('新输入')
    await editor.finishText()
    expect(editor.history.present[0].text).toBe('新输入')
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
