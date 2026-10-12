import { describe, expect, it } from 'vitest'
import { newerRecording, recordingActive, recordingTime, type RecordingView } from '../src/lib/recording'

const view: RecordingView = { id: 7, revision: 3, phase: 'paused', seconds: 23, frames: 180, message: '', path: null }
describe('录屏会话状态', () => {
  it('暂停、结束后不被迟到的采集查询覆盖', () => {
    expect(newerRecording(view, { ...view, revision: 2, phase: 'recording' })).toBe(view)
    const completed: RecordingView = { ...view, revision: 5, phase: 'completed' }
    expect(newerRecording(completed, view)).toBe(completed)
  })
  it('新会话接受较小修订号，拒绝旧会话及空查询', () => {
    const next = { ...view, id: 8, revision: 1 }
    expect(newerRecording(view, next)).toBe(next)
    expect(newerRecording(next, view)).toBe(next)
    expect(newerRecording(next, null)).toBe(next)
    expect(newerRecording(null, null)).toBeNull()
  })
  it('编码完成前保持占用，终态允许重新录制', () => {
    for (const phase of ['ready', 'recording', 'paused', 'encoding'] as const) expect(recordingActive({ ...view, phase })).toBe(true)
    for (const phase of ['completed', 'cancelled', 'failed'] as const) expect(recordingActive({ ...view, phase })).toBe(false)
    expect(recordingActive(null)).toBe(false)
  })
  it('使用后端时间展示，边界不四舍五入到下一分钟', () => {
    expect(recordingTime(59.99)).toBe('00:59')
    expect(recordingTime(180)).toBe('03:00')
    expect(recordingTime(-1)).toBe('00:00')
  })
  it('待开始保留占用，开始后不接受迟到的待开始查询', () => {
    const ready: RecordingView = { ...view, phase: 'ready', revision: 1, seconds: 0, frames: 0 }
    const running: RecordingView = { ...ready, phase: 'recording', revision: 2 }
    expect(recordingActive(ready)).toBe(true)
    expect(newerRecording(ready, running)).toBe(running)
    expect(newerRecording(running, ready)).toBe(running)
  })
})
