import { beforeEach, describe, expect, it, vi } from 'vitest'
import { hitTest, magnifierLens, magnifierSource, shapeBounds, spotlightPath, type Annotation } from '../../src/features/annotations/annotations'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { prepareRaster } from '../../src/features/annotations/annotation-raster'

vi.mock('../../src/features/annotations/annotation-raster', () => ({ prepareRaster: vi.fn() }))
const bounds = () => ({ x: 20, y: 30, width: 200, height: 120 })
const lens: Annotation = { kind: 'magnify', start: { x: 40, y: 50 }, end: { x: 120, y: 110 }, color: '#ff0000', width: 3, magnification: 2 }
beforeEach(() => { vi.mocked(prepareRaster).mockReset().mockImplementation(async shape => {
  const b = shapeBounds(shape)
  return { ...shape, start: { x: b.x, y: b.y }, end: { x: b.x + b.width, y: b.y + b.height }, raster: 'effect' }
}) })
function editorFor(tool: 'magnify' | 'spotlight', persist?: Parameters<typeof useAnnotations>[3]) {
  const editor = useAnnotations(bounds, () => 'source', () => false, persist)
  editor.setTool(tool)
  return editor
}
async function draw(editor: ReturnType<typeof editorFor>) {
  editor.begin(lens.end, 2); editor.drag(lens.start); await editor.finish()
}

describe('spotlight and magnifier geometry', () => {
  it('samples the centered region and handles reverse drag and zoom changes', () => {
    expect(magnifierSource(lens)).toEqual({ x: 60, y: 65, width: 40, height: 30 })
    expect(magnifierSource({ ...lens, start: lens.end, end: lens.start, magnification: 4 })).toEqual({ x: 70, y: 72.5, width: 20, height: 15 })
    for (const magnification of [NaN, Infinity, 0, 1, 5]) expect(() => magnifierSource({ ...lens, magnification })).toThrow()
  })
  it('expands the lens around the dragged region and keeps it inside the crop', () => {
    const crop = bounds()
    expect(magnifierLens({ x: 100, y: 80, width: 40, height: 20 }, 2, crop)).toEqual({ x: 80, y: 70, width: 80, height: 40 })
    // 贴近左上角：镜框平移回选区内，尺寸不变。
    expect(magnifierLens({ x: 20, y: 30, width: 40, height: 20 }, 3, crop)).toEqual({ x: 20, y: 30, width: 120, height: 60 })
    // 展开后比选区还大：等比缩到刚好放下。
    expect(magnifierLens({ x: 60, y: 50, width: 100, height: 60 }, 4, crop)).toEqual({ x: 20, y: 30, width: 200, height: 120 })
    for (const zoom of [NaN, 1, 5]) expect(() => magnifierLens({ x: 0, y: 0, width: 10, height: 10 }, zoom, crop)).toThrow()
  })
  it('limits the spotlight path to the crop and selects its bright region, not the entire shade', () => {
    const shape = { ...lens, kind: 'spotlight' as const, start: lens.end, end: lens.start }
    expect(spotlightPath(shape, bounds())).toBe('M 20 30 h 200 v 120 h -200 Z M 40 50 h 80 v 60 h -80 Z')
    expect(hitTest([shape], { x: 80, y: 80 }, 0)).toBe(0)
    expect(hitTest([shape], { x: 200, y: 140 }, 0)).toBe(-1)
  })
})
describe('shared effect editor', () => {
  it('shares spotlight style across bright regions and undoes the group update once', async () => {
    const editor = editorFor('spotlight')
    await draw(editor); await draw(editor)
    editor.setTool('edit'); editor.select(1)
    await editor.style({ color: '#123456', opacity: .35 })
    expect(editor.history.present.map(s => [s.color, s.opacity])).toEqual([['#123456', .35], ['#123456', .35]])
    editor.setTool('spotlight')
    expect(editor.opacity).toBe(.35)
    await draw(editor)
    expect(editor.history.present[2].color).toBe('#123456')
    await editor.travel('undo'); await editor.travel('undo')
    expect(editor.history.present.map(s => s.opacity)).toEqual([.7, .7])
    await editor.style({ opacity: .15 })
    expect(editor.history.present.map(s => s.opacity)).toEqual([.15, .15])
  })
  it('commits a spotlight without raster data and supports style, undo and redo', async () => {
    const editor = editorFor('spotlight')
    await draw(editor)
    expect(editor.history.present[0]).toMatchObject({ kind: 'spotlight', color: '#000000', opacity: .7 })
    expect(prepareRaster).not.toHaveBeenCalled()
    editor.setTool('edit'); editor.select(0); await editor.style({ opacity: .35, color: '#112233' })
    await editor.travel('undo'); expect(editor.history.present[0].opacity).toBe(.7)
    await editor.travel('redo'); expect(editor.history.present[0].opacity).toBe(.35)
  })
  it('magnifies the whole dragged region and keeps it when the magnification changes', async () => {
    // 框出 40×20 的内容：2 倍镜框围绕它展开为 80×40，采样区域正是框出的那一块。
    const editor = editorFor('magnify')
    editor.begin({ x: 140, y: 100 }, 2); editor.drag({ x: 100, y: 80 }); await editor.finish()
    const drawn = editor.history.present[0]
    expect(drawn).toMatchObject({ kind: 'magnify', start: { x: 80, y: 70 }, end: { x: 160, y: 110 }, magnification: 2, raster: 'effect' })
    expect(magnifierSource(drawn)).toEqual({ x: 100, y: 80, width: 40, height: 20 })
    // 之后移动、缩放的是镜框本身。
    editor.setTool('edit'); editor.begin({ x: 120, y: 90 }, 2); editor.drag({ x: 130, y: 100 }); await editor.finish()
    expect(editor.history.present[0].start).toEqual({ x: 90, y: 80 })
    editor.begin({ x: 170, y: 120 }, 2, 'se'); editor.drag({ x: 190, y: 140 }); await editor.finish()
    expect(editor.history.present[0].end).toEqual({ x: 190, y: 140 })
    // 改倍率：被放大的内容（115,95 起 50×30）不变，镜框重新展开并收回截图选区内。
    await editor.style({ magnification: 4 })
    expect(editor.history.present[0]).toMatchObject({ magnification: 4, start: { x: 20, y: 30 }, end: { x: 220, y: 150 } })
    expect(prepareRaster).toHaveBeenCalledTimes(4)
    await editor.style({ magnification: NaN })
    expect(editor.history.present[0].magnification).toBe(4)
  })
  it('rejects empty drags and can cancel without committing', async () => {
    for (const tool of ['spotlight', 'magnify'] as const) {
      const editor = editorFor(tool)
      editor.begin(lens.start, 2); await editor.finish()
      editor.begin(lens.start, 2); editor.drag(lens.end); editor.abort()
      expect(editor.history.present).toEqual([])
    }
  })
  it('preserves the previous image and annotation when resampling or persistence fails', async () => {
    const persist = vi.fn().mockResolvedValue(undefined), editor = editorFor('magnify', persist)
    await draw(editor); const original = editor.history.present[0]
    editor.setTool('edit'); editor.select(0)
    vi.mocked(prepareRaster).mockRejectedValueOnce(new Error('decode failed'))
    await editor.style({ magnification: 3 })
    expect(editor.history.present[0]).toEqual(original)
    persist.mockRejectedValueOnce(new Error('native failed'))
    await editor.style({ magnification: 4 })
    expect(editor.history.present[0]).toEqual(original)
    expect(editor.pending).toBe(false)
  })
})
