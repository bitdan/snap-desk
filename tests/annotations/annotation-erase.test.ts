import { describe, expect, it, vi } from 'vitest'
import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import AnnotationStack from '../../src/features/annotations/components/AnnotationStack'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { hitTest, mapShape, shapePath, type Annotation } from '../../src/features/annotations/annotations'

const bounds = () => ({ x: 10, y: 20, width: 100, height: 100 })
const pen: Annotation = { kind: 'pen', start: { x: 20, y: 30 }, end: { x: 80, y: 30 }, points: [{ x: 20, y: 30 }, { x: 80, y: 30 }], width: 10, color: '#ff0000' }
const make = (persist?: (shapes: Annotation[]) => Promise<void>) => useAnnotations(bounds, () => '', () => false, persist)

describe('局部像素橡皮擦', () => {
  it('预览整体遮罩且保留后画图层', async () => {
    const erase: Annotation = { ...pen, kind: 'erase' }
    const html = await renderToString(createSSRApp(() => h(AnnotationStack, {
      shapes: [pen, pen, erase, pen], maskPrefix: 'test',
    }, { default: ({ index }: { index: number }) => h('path', { id: `shape-${index}` }) })))
    expect(html).toBe('<g><g mask="url(#test-erase-2)"><path id="shape-0"></path><path id="shape-1"></path></g><path id="shape-3"></path></g>')
  })
  it('单击圆点及坐标变换', async () => {
    const editor = make(); editor.reset([pen]); editor.setTool('erase')
    editor.begin({ x: 50, y: 30 }, 1); await editor.finish()
    const erase = editor.history.present[1]
    expect(erase.kind).toBe('erase'); expect(erase.width).toBe(24)
    expect(erase.points).toEqual([{ x: 50, y: 30 }])
    expect(shapePath(erase)).toBe('M 50 30 h .001')
    expect(hitTest([erase], { x: 55, y: 30 }, 0)).toBe(0)
    const moved = mapShape(erase, p => ({ x: p.x - 10, y: p.y - 20 }))
    expect(moved.points).toEqual([{ x: 40, y: 10 }])
  })
  it('拖动提交一次并支持撤销重做', async () => {
    const persist = vi.fn().mockResolvedValue(undefined), editor = make(persist)
    editor.reset([pen]); editor.setTool('erase'); await editor.style({ width: 32 })
    editor.begin({ x: 50, y: 25 }, 1); editor.drag({ x: 50, y: 80 }); editor.drag({ x: 200, y: 200 })
    expect(persist).not.toHaveBeenCalled(); await editor.finish()
    expect(persist).toHaveBeenCalledTimes(1)
    expect(editor.history.present[1].points?.at(-1)).toEqual({ x: 110, y: 120 })
    expect(editor.history.present[0]).toEqual(pen)
    await editor.travel('undo'); expect(editor.history.present).toEqual([pen])
    await editor.travel('redo'); expect(editor.history.present[1].width).toBe(32)
    editor.setTool('line'); editor.begin({ x: 20, y: 25 }, 1, undefined, true); editor.drag({ x: 80, y: 25 }); await editor.finish()
    expect(editor.history.present.map(s => s.kind)).toEqual(['pen', 'erase', 'line'])
  })
  it('取消及持久化失败回滚', async () => {
    const editor = make(async () => { throw new Error('保存失败') })
    editor.reset([pen]); editor.setTool('erase'); editor.begin({ x: 50, y: 30 }, 1); editor.abort()
    expect(editor.history.present).toEqual([pen])
    editor.begin({ x: 50, y: 30 }, 1); await editor.finish()
    expect(editor.history.present).toEqual([pen]); expect(editor.history.past).toEqual([])
    expect(editor.error).toContain('保存失败')
  })
  it('整项删除仍可撤销', async () => {
    const editor = make(); editor.reset([pen]); editor.setTool('eraser'); editor.begin({ x: 50, y: 30 }, 1)
    await vi.waitFor(() => expect(editor.history.present).toEqual([]))
    await editor.travel('undo'); expect(editor.history.present).toEqual([pen])
  })
})
