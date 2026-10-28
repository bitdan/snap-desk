import { describe, expect, it } from 'vitest'
import { annotationAlpha, annotationsAt, prepareRecordingRaster, type TimedAnnotation } from '../../src/features/recording/recording-annotations'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import type { Annotation } from '../../src/features/annotations/annotations'

const bounds = { x: 0, y: 0, width: 640, height: 480 }
describe('录制中的标注', () => {
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

describe('标注媒体时间轨道', () => {
  const track:TimedAnnotation={id:'a-1',shape:{kind:'rectangle',start:{x:0,y:0},end:{x:10,y:10},color:'#ff0000',width:2,opacity:.5},startMs:1000,endMs:4000,fadeOutMs:500,order:0}
  it('起止采用半开区间，淡出不改变原形状',()=>{
    expect(annotationsAt([track],999)).toEqual([])
    expect(annotationAlpha(track,1000)).toBe(1)
    expect(annotationsAt([track],3750)[0].opacity).toBe(.25)
    expect(annotationsAt([track],4000)).toEqual([])
    expect(track.shape.opacity).toBe(.5)
  })
  it('永久标注不会依赖系统墙钟，短片段淡出不超过片段长度',()=>{
    expect(annotationAlpha({...track,endMs:null},1e9)).toBe(1)
    expect(annotationAlpha({...track,startMs:3900},3900)).toBe(1)
    expect(annotationAlpha({...track,startMs:3900},3950)).toBe(.5)
  })
})
