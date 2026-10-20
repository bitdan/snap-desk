import { describe, expect, it } from 'vitest'
import { newerScroll, scrollViewportStyle } from '../src/lib/scroll-capture'
import type { ScrollView } from '../src/lib/api'
const view: ScrollView = { id: 3, revision: 2, collecting: false,
  archivePath: null, width: 800, height: 1800, frames: 4, message: 'finished', preview: '', frameHeight: 600,
  canvasBounds: { x: -400, y: -600, width: 800, height: 1800 },
  viewportRect: { x: 0, y: 600, width: 400, height: 600 },
  coveragePercent: 80, hasHoles: true, direction: 'down', status: 'finishing' }
describe('scroll completion state', () => {
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
    const recovering = { ...collecting, revision: 3, message: '请稍微向上滚动，匹配后自动继续。' }
    expect(newerScroll(collecting, recovering, 3)).toBe(recovering)
    expect(newerScroll(recovering, collecting, 3)).toBe(recovering)
    const resumed = { ...recovering, revision: 4, frames: 5, height: 2100, message: '自动拼接中' }
    expect(newerScroll(recovering, resumed, 3)).toBe(resumed)
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
