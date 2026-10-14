import { describe, expect, it } from 'vitest'
import { newerScroll } from '../src/lib/scroll-capture'
import type { ScrollView } from '../src/lib/api'
const view: ScrollView = { id: 3, revision: 2, collecting: false,
  archivePath: null, width: 800, height: 1800, frames: 4, message: 'finished', preview: '', frameHeight: 600 }
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
})
