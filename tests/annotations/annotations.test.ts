import { describe, expect, it } from 'vitest'
import { append, arrowWings, commit, emptyHistory, hitTest, imagePoint, moveShape, resizeShape, shapeBounds, redo, shapePath, undo, HISTORY_RASTER_BYTES, type Annotation } from '../../src/features/annotations/annotations'

const rectangle: Annotation = { kind: 'rectangle', start: { x: 40, y: 30 }, end: { x: 10, y: 5 }, color: '#ef4444', width: 3 }
describe('annotations', () => {
  it('evicts old raster versions by unique memory while sharing unchanged layers', () => {
    const raster = 'x'.repeat(2_000_000)
    let history = emptyHistory([{ ...rectangle, raster }])
    for (let i = 0; i < 60; i++) history = commit(history, [{ ...rectangle, width: i + 1, raster }])
    expect(history.past).toHaveLength(50)
    const before = history
    for (let i = 0; i < 12; i++) history = commit(history, [{ ...rectangle, raster: `${i}${raster}` }])
    const unique = new Set([...history.past, history.present].flatMap(shapes => shapes.map(shape => shape.raster!)))
    expect([...unique].reduce((total, value) => total + value.length * 2, 0)).toBeLessThanOrEqual(HISTORY_RASTER_BYTES)
    expect(history.past.length).toBeLessThan(12)
    expect(history.present[0].raster).toBe(`11${raster}`)
    expect(redo(undo(history))).toEqual(history)
    expect(before.past).toHaveLength(50)
  })
  it('maps scaled pointer coordinates and clips strokes at the crop boundary', () => {
    const viewport = { width: 800, height: 600 }, image = { width: 1600, height: 1200 }
    const crop = { x: 100, y: 200, width: 600, height: 400 }
    expect(imagePoint({ x: 200, y: 150 }, viewport, image, crop)).toEqual({ x: 400, y: 300 })
    expect(imagePoint({ x: -10, y: 900 }, viewport, image, crop)).toEqual({ x: 100, y: 600 })
  })
  it('undoes and redoes in drawing order without mutating prior state', () => {
    const first = append(emptyHistory(), rectangle)
    const second = append(first, { ...rectangle, kind: 'arrow' })
    expect(undo(second).present).toEqual(first.present)
    expect(redo(undo(second))).toEqual(second)
    expect(first.present).toHaveLength(1)
    expect(undo(undo(second)).present).toEqual([])
  })
  it('clears redo after a new drawing', () => {
    const h = append(undo(append(emptyHistory(), rectangle)), { ...rectangle, color: '#ffffff' })
    expect(h.future).toEqual([])
    expect(redo(h)).toBe(h)
  })
  it('keeps rectangle drag direction and closes the path', () => {
    expect(shapePath(rectangle)).toBe('M 40 30 H 10 V 5 H 40 Z')
  })
  it('previews lines without arrow wings and ellipses independent of drag direction', () => {
    expect(shapePath({ ...rectangle, kind: 'line' })).toBe('M 40 30 L 10 5')
    const ellipse: Annotation = { ...rectangle, kind: 'ellipse' }
    expect(shapePath(ellipse)).toBe('M 10 17.5 A 15 12.5 0 1 0 40 17.5 A 15 12.5 0 1 0 10 17.5 Z')
    expect(shapePath({ ...ellipse, start: ellipse.end, end: ellipse.start })).toBe(shapePath(ellipse))
  })
  it('produces arrow wings using the native compositor geometry', () => {
    const arrow: Annotation = { ...rectangle, kind: 'arrow', start: { x: 0, y: 20 }, end: { x: 30, y: 20 }, width: 2 }
    expect(arrowWings(arrow)).toEqual([{ x: 20, y: 15 }, { x: 20, y: 25 }])
    expect(arrowWings({ ...arrow, end: arrow.start })).toEqual([arrow.start, arrow.start])
  })
  it('undoes moves, restyling and deletion as complete transactions', () => {
    const original = append(emptyHistory(), rectangle)
    const moved = commit(original, [{ ...rectangle, color: '#ffffff', start: { x: 30, y: 20 } }])
    const deleted = commit(moved, [])
    expect(undo(deleted).present).toEqual(moved.present)
    expect(undo(undo(deleted)).present).toEqual(original.present)
    expect(redo(undo(deleted))).toEqual(deleted)
    expect(commit(undo(deleted), [rectangle]).future).toEqual([])
  })
  it('moves every pen point and clamps the whole stroke within the crop', () => {
    const pen: Annotation = { ...rectangle, kind: 'pen', start: { x: 20, y: 20 }, end: { x: 40, y: 40 }, points: [{ x: 20, y: 20 }, { x: 30, y: 50 }, { x: 40, y: 40 }] }
    const moved = moveShape(pen, { x: 100, y: -100 }, { x: 10, y: 10, width: 60, height: 60 })
    expect(moved.points).toEqual([{ x: 50, y: 10 }, { x: 60, y: 40 }, { x: 70, y: 30 }])
    expect(pen.points![0]).toEqual({ x: 20, y: 20 })
    expect(shapeBounds(moved)).toEqual({ x: 50, y: 10, width: 20, height: 30 })
  })
  it('resizes reverse shapes and line endpoints without NaN', () => {
    const bounds = { x: 0, y: 0, width: 100, height: 100 }
    expect(shapeBounds(resizeShape(rectangle, 'se', { x: 90, y: 80 }, bounds))).toEqual({ x: 10, y: 5, width: 80, height: 75 })
    const line: Annotation = { ...rectangle, kind: 'line', start: { x: 5, y: 10 }, end: { x: 50, y: 10 } }
    expect(resizeShape(line, 'end', { x: 120, y: -10 }, bounds).end).toEqual({ x: 100, y: 0 })
  })
  it('hit tests topmost strokes, arrow wings and raster interiors', () => {
    const line: Annotation = { ...rectangle, kind: 'line', start: { x: 0, y: 20 }, end: { x: 30, y: 20 }, width: 2 }
    expect(hitTest([line, { ...line, color: '#ffffff' }], { x: 10, y: 20 }, 1)).toBe(1)
    expect(hitTest([line], { x: 10, y: 35 }, 1)).toBe(-1)
    expect(hitTest([{ ...line, kind: 'arrow' }], { x: 20, y: 15 }, 1)).toBe(0)
    expect(hitTest([{ ...rectangle, kind: 'text' }], { x: 20, y: 20 }, 1)).toBe(0)
    expect(hitTest([rectangle], { x: 20, y: 20 }, 1)).toBe(-1)
  })
})
