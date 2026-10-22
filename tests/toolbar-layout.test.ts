import { afterEach, describe, expect, it, vi } from 'vitest'
import { clampToolbar } from '../src/lib/toolbar-layout'
import { moveTool, normalizeToolOrder, readToolOrder } from '../src/lib/toolbar-order'

afterEach(() => vi.unstubAllGlobals())

describe('工具栏位置与顺序', () => {
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
