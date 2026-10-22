import { describe, expect, it } from 'vitest'
import { newerScroll, newerScrollPreview, scrollViewportStyle } from '../src/lib/scroll-capture'
import type { ScrollView } from '../src/lib/api'
const view: ScrollView = { id: 3, revision: 2, collecting: false,
  archivePath: null, width: 800, height: 1800, frames: 4, message: 'finished', preview: '', frameHeight: 600,
  canvasBounds: { x: -400, y: -600, width: 800, height: 1800 },
  viewportRect: { x: 0, y: 600, width: 400, height: 600 },
  coveragePercent: 80, hasHoles: true, direction: 'down', status: 'finishing' }
describe('scroll completion state', () => {
  it('关闭后拒绝迟到预览和挂起查询，但允许下一会话', () => {
    expect(newerScrollPreview(null, view, 3)).toBeNull()
    expect(newerScrollPreview(null, { ...view, revision: 99 }, 3)).toBeNull()
    const next = { ...view, id: 4, revision: 1, collecting: true }
    expect(newerScrollPreview(null, next, 3)).toBe(next)
    expect(newerScrollPreview(next, { ...view, revision: 100 }, 3)).toBe(next)
    expect(newerScrollPreview({ ...next, revision: 2 }, next, 3)?.revision).toBe(2)
  })
  it('does not overwrite completion with a delayed start response', () => {
    expect(newerScroll(view, { ...view, revision: 1, collecting: true }, 3)).toBe(view)
  })
  it('ignores completion for cancelled or replaced sessions', () => {
    expect(newerScroll(null, view, undefined)).toBeNull()
    expect(newerScroll(null, view, 4)).toBeNull()
  })
  it('accepts completion without seam fields or user action', () => {
    expect(newerScroll({ ...view, revision: 1, collecting: true }, view, 3)).toBe(view)
  })
  it('accepts recovery hints without ending capture and ignores stale progress', () => {
    const collecting = { ...view, collecting: true }
    const recovering = { ...collecting, revision: 3, status: 'recovering' as const, message: '请沿原路滚回重叠区，匹配后自动继续。' }
    expect(newerScroll(collecting, recovering, 3)).toBe(recovering)
    expect(newerScroll(recovering, collecting, 3)).toBe(recovering)
    const unsettled = { ...recovering, revision: 4, message: '已确认内容保留；请沿原路滚回重叠区并停顿。' }
    expect(newerScroll(recovering, unsettled, 3)?.status).toBe('recovering')
    expect(unsettled.preview).toBe(recovering.preview)
    expect(unsettled.frames).toBe(recovering.frames)
    const resumed = { ...unsettled, revision: 5, status: 'collecting' as const, frames: 5, height: 2100, message: '自动拼接中' }
    expect(newerScroll(unsettled, resumed, 3)).toBe(resumed)
  })
})

describe('二维滚动预览', () => {
  it('按负坐标包围盒归一化视口', () => {
    const style = scrollViewportStyle(view)
    expect(style.left).toBe('50%')
    expect(style.width).toBe('50%')
    expect(parseFloat(style.top)).toBeCloseTo(66.6667)
    expect(parseFloat(style.height)).toBeCloseTo(33.3333)
  })
  it('回滚只移动视口，不假定位置在底部', () => {
    const back = { ...view, revision: 3, viewportRect: { x: -400, y: -600, width: 400, height: 600 }, direction: 'up' as const }
    expect(newerScroll(view, back, 3)).toBe(back)
    expect(scrollViewportStyle(back).top).toBe('0%')
    expect(scrollViewportStyle(back).left).toBe('0%')
    expect(back.coveragePercent).toBe(view.coveragePercent)
    expect(newerScroll(back, view, 3)).toBe(back)
  })
  it('视口裁到画布且不对空画布除零', () => {
    expect(scrollViewportStyle({ canvasBounds: { x: 0, y: 0, width: 100, height: 100 },
      viewportRect: { x: -50, y: 80, width: 100, height: 50 } })).toEqual({ left: '0%', top: '80%', width: '50%', height: '20%' })
    expect(scrollViewportStyle({ ...view, canvasBounds: { x: 0, y: 0, width: 0, height: 0 } }).width).toBe('0%')
  })
  it('保存失败作为显式状态传播，不由提示文字判断', () => {
    const failed = { ...view, revision: 4, status: 'failed' as const, message: '磁盘写入失败' }
    expect(newerScroll(view, failed, 3)?.status).toBe('failed')
  })
})
