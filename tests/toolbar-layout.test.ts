import { afterEach, describe, expect, it, vi } from 'vitest'
import { annotationToolbarPosition, clampToolbar } from '../src/lib/toolbar-layout'
import { moveTool, normalizeToolOrder, readToolOrder } from '../src/lib/toolbar-order'

afterEach(() => vi.unstubAllGlobals())

describe('工具栏位置与顺序', () => {
  it('截图和录像的工具栏都跟随选区，展开变高后自动避让屏幕边缘', () => {
    const selection = { x: 850, y: 500, width: 100, height: 40 }
    const display = { x: 0, y: 0, width: 1000, height: 700 }
    expect(annotationToolbarPosition(selection, display, { width: 600, height: 60 })).toEqual({ left: '392px', top: '552px', maxWidth: '984px' })
    expect(annotationToolbarPosition(selection, display, { width: 600, height: 200 }).top).toBe('492px')
    expect(annotationToolbarPosition(selection, display, { width: 1200, height: 800 })).toEqual({ left: '8px', top: '8px', maxWidth: '984px' })
  })
  it('浮动位置限制在视口内，窗口缩小后仍可找到手柄', () => {
    const size = { width: 700, height: 80 }
    expect(clampToolbar({ x: 1000, y: -20 }, size, { width: 1200, height: 800 })).toEqual({ x: 500, y: 0 })
    expect(clampToolbar({ x: 500, y: 720 }, size, { width: 400, height: 600 })).toEqual({ x: 0, y: 520 })
  })
  it('恢复时过滤损坏数据，保留顺序并补齐新增工具', () => {
    const defaults = normalizeToolOrder(null)
    const restored = normalizeToolOrder(['text', 'unknown', 'text', null, 'arrow'])
    expect(restored.slice(0, 2)).toEqual(['text', 'arrow'])
    expect(new Set(restored)).toEqual(new Set(defaults))
    expect(restored).toHaveLength(defaults.length)
    expect(normalizeToolOrder({ tool: 'text' })).toEqual(defaults)
  })
  it('左右移动保持工具完整且不修改输入', () => {
    const order = normalizeToolOrder(null)
    const moved = moveTool(order, 'text', 'edit')
    expect(moved[0]).toBe('text')
    expect(order[0]).toBe('edit')
    expect(moveTool(moved, 'text', 'text')).toEqual(moved)
    expect(moveTool(moved, 'text', 'eraser').at(-1)).toBe('text')
    expect(new Set(moved)).toEqual(new Set(order))
  })
  it('本地存储不可用或 JSON 损坏时回退默认顺序', () => {
    vi.stubGlobal('localStorage', { getItem: () => '{broken' })
    expect(readToolOrder()).toEqual(normalizeToolOrder(null))
    vi.stubGlobal('localStorage', { getItem: () => { throw new Error('denied') } })
    expect(readToolOrder()).toEqual(normalizeToolOrder(null))
    vi.stubGlobal('localStorage', { getItem: () => '["text","arrow"]' })
    expect(readToolOrder().slice(0, 2)).toEqual(['text', 'arrow'])
  })
})
