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
  it('accepts recovery hints without ending capture and ignores stale progress', () => {
    const collecting = { ...view, collecting: true }
    const recovering = { ...collecting, revision: 3, message: '请稍微向上滚动，匹配后自动继续。' }
    expect(newerScroll(collecting, recovering, 3)).toBe(recovering)
    expect(newerScroll(recovering, collecting, 3)).toBe(recovering)
    const resumed = { ...recovering, revision: 4, frames: 5, height: 2100, message: '自动拼接中' }
    expect(newerScroll(recovering, resumed, 3)).toBe(resumed)
  })
})
