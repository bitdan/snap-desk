import { describe, expect, it, vi } from 'vitest'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import type { Annotation } from '../../src/features/annotations/annotations'

const bounds = () => ({ x: 0, y: 0, width: 200, height: 100 })
const rect: Annotation = { kind: 'rectangle', start: { x: 10, y: 10 }, end: { x: 50, y: 50 }, color: '#ff0000', width: 3 }

describe('shared screenshot and pin editor', () => {
  it('commits only once per drag and supports editing imported annotations', async () => {
    const persist = vi.fn().mockResolvedValue(undefined)
    const editor = useAnnotations(bounds, () => '', () => false, persist)
    editor.reset([rect])
    editor.begin({ x: 10, y: 20 }, 2)
    editor.drag({ x: 20, y: 20 }); editor.drag({ x: 40, y: 20 })
    expect(persist).not.toHaveBeenCalled()
    await editor.finish()
    expect(persist).toHaveBeenCalledTimes(1)
    expect(editor.history.present[0].start.x).toBe(40)
    expect(editor.shapes).toBe(editor.history.present)
    expect(editor.canUndo).toBe(true)
    expect(editor.canRedo).toBe(false)
    await editor.travel('undo')
    expect(editor.history.present).toEqual([rect])
    expect(editor.canUndo).toBe(false)
    expect(editor.canRedo).toBe(true)
    await editor.travel('redo')
    expect(editor.history.present[0].start.x).toBe(40)
    expect(editor.canUndo).toBe(true)
    expect(editor.canRedo).toBe(false)
  })
  it('keeps old annotations and history when the native update fails', async () => {
    const editor = useAnnotations(bounds, () => '', () => false, async () => { throw new Error('write failed') })
    editor.reset([rect]); editor.select(0)
    await editor.style({ color: '#ffffff' })
    expect(editor.history.present).toEqual([rect])
    expect(editor.history.past).toEqual([])
    expect(editor.color).toBe('#ff0000')
    expect(editor.error).toContain('write failed')
  })
  it('cancels an incomplete stroke without modifying history', () => {
    const editor = useAnnotations(bounds, () => '', () => false)
    editor.setTool('pen'); editor.begin({ x: 10, y: 10 }, 2); editor.drag({ x: 80, y: 30 }); editor.abort()
    expect(editor.history.present).toEqual([])
    expect(editor.interacting).toBe(false)
  })
  it('finishes a polyline only on explicit completion and supports deleting it', async () => {
    const editor = useAnnotations(bounds, () => '', () => false)
    editor.setTool('polyline'); editor.begin({ x: 10, y: 10 }, 2); await editor.finish()
    editor.begin({ x: 40, y: 20 }, 2); editor.begin({ x: 60, y: 5 }, 2)
    expect(editor.history.present).toEqual([])
    await editor.finish(true)
    expect(editor.history.present[0].points).toHaveLength(3)
    editor.setTool('edit'); editor.select(0); await editor.remove()
    expect(editor.history.present).toEqual([])
    await editor.travel('undo')
    expect(editor.history.present[0].kind).toBe('polyline')
  })
  it('blocks changes while locked or a native update is pending', async () => {
    let locked = true, release!: () => void
    const editor = useAnnotations(bounds, () => '', () => locked, () => new Promise<void>(resolve => { release = resolve }))
    editor.reset([rect]); editor.select(0)
    await editor.remove(); expect(editor.history.present).toHaveLength(1)
    locked = false
    const task = editor.remove()
    expect(editor.pending).toBe(true)
    editor.begin({ x: 30, y: 30 }, 2)
    expect(editor.interacting).toBe(false)
    release(); await task
    expect(editor.history.present).toEqual([])
  })
  it('clears all objects in one undoable operation and ignores a click without movement', async () => {
    const editor = useAnnotations(bounds, () => '', () => false)
    editor.reset([rect]); editor.begin({ x: 10, y: 20 }, 2); await editor.finish()
    expect(editor.history.past).toEqual([])
    await editor.clear(); expect(editor.history.present).toEqual([])
    await editor.travel('undo'); expect(editor.history.present).toEqual([rect])
  })
  it('rejects zero-area resize of a text object without losing the original', async () => {
    const editor = useAnnotations(bounds, () => '', () => false)
    const text: Annotation = { ...rect, kind: 'text', raster: 'unused', text: 'hello', fontSize: 20, fontFamily: 'sans' }
    editor.reset([text]); editor.select(0)
    editor.begin(text.end, 2, 'se'); editor.drag(text.start); await editor.finish()
    expect(editor.history.present).toEqual([text])
    expect(editor.history.past).toEqual([])
  })
  it('does not restore an old async edit after a new image is loaded', async () => {
    let release!: () => void
    const editor = useAnnotations(bounds, () => '', () => false, () => new Promise<void>(resolve => { release = resolve }))
    editor.reset([rect]); editor.select(0)
    const task = editor.remove()
    const next = { ...rect, color: '#ffffff' }
    editor.reset([next]); release(); await task
    expect(editor.history.present).toEqual([next])
  })
})
