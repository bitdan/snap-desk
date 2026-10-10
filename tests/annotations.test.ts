import { describe, expect, it } from 'vitest'
import { append, arrowWings, emptyHistory, imagePoint, redo, shapePath, undo, type Annotation } from '../src/lib/annotations'

const rectangle: Annotation = { kind: 'rectangle', start: { x: 40, y: 30 }, end: { x: 10, y: 5 }, color: '#ef4444', width: 3 }
describe('annotations', () => {
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
  it('produces arrow wings using the native compositor geometry', () => {
    const arrow: Annotation = { ...rectangle, kind: 'arrow', start: { x: 0, y: 20 }, end: { x: 30, y: 20 }, width: 2 }
    expect(arrowWings(arrow)).toEqual([{ x: 20, y: 15 }, { x: 20, y: 25 }])
    expect(arrowWings({ ...arrow, end: arrow.start })).toEqual([arrow.start, arrow.start])
  })
})
