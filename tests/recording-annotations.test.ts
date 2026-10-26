import { describe, expect, it } from 'vitest'
import { prepareRecordingRaster, recordingToolbarRegion } from '../src/lib/recording-annotations'
import { useAnnotations } from '../src/lib/use-annotations'
import type { Annotation } from '../src/lib/annotations'

const bounds = { x: 0, y: 0, width: 640, height: 480 }
describe('录制中的标注', () => {
  it('工具栏命中区域按 DPI 转成物理像素，边缘不越界', () => {
    expect(recordingToolbarRegion({ x: 100, y: 200, width: 300, height: 80 }, { width: 1280, height: 720 }, { width: 1920, height: 1080 }))
      .toEqual({ x: 150, y: 300, width: 450, height: 120 })
    expect(recordingToolbarRegion({ x: -10, y: 700, width: 1300, height: 100 }, { width: 1280, height: 720 }, { width: 1920, height: 1080 }))
      .toEqual({ x: 0, y: 1050, width: 1920, height: 30 })
  })
  it.each(['mosaic', 'blur', 'magnify'] as const)('%s 只提交范围，不从旧截图冻结像素', async kind => {
    const shape: Annotation = { kind, start: { x: 100, y: 90 }, end: { x: 20, y: 10 }, color: '#ff0000', width: 12, opacity: .35, raster: 'old-pixels' }
    const next = await prepareRecordingRaster(shape, 'unavailable-screenshot', bounds)
    expect(next.start).toEqual({ x: 20, y: 10 })
    expect(next.end).toEqual({ x: 100, y: 90 })
    expect(next.raster).toBeUndefined()
    expect(next.opacity).toBe(kind === 'magnify' ? .35 : 1)
  })
  it('完成笔画后才持久化，支持撤销重做；失败保留已有图形', async () => {
    const writes: Annotation[][] = []
    let reject = false
    const editor = useAnnotations(() => bounds, () => '', () => false, async shapes => {
      if (reject) throw new Error('录制已结束')
      writes.push(shapes)
    }, prepareRecordingRaster)
    editor.setTool('mosaic')
    editor.begin({ x: 10, y: 10 }, 1)
    editor.drag({ x: 100, y: 100 })
    expect(writes).toHaveLength(0)
    await editor.finish()
    expect(writes[0][0].kind).toBe('mosaic')
    expect(writes[0][0].raster).toBeUndefined()
    await editor.travel('undo')
    expect(writes[1]).toEqual([])
    await editor.travel('redo')
    expect(editor.history.present).toHaveLength(1)
    reject = true
    await editor.clear()
    expect(editor.history.present).toHaveLength(1)
    expect(editor.error).toContain('录制已结束')
  })
})
