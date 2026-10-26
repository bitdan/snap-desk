/** 与原生录屏编辑器契约一致：毫秒整数，路径只展示，所有动作按原生 token 执行。 */
export interface RecordingEditorView { token: number; format: 'mp4' | 'gif'; durationMs: number; previewAvailable: boolean }
export interface RecordingTrimRange { startMs: number; endMs: number }
export interface RecordingTrimExport { path: string; durationMs: number; previewAvailable: boolean }

/** 拖动端点时限制为连续、至少 20ms 的片段；GIF 使用 10ms 的帧时钟。 */
export function moveRecordingTrim(range: RecordingTrimRange, edge: 'start' | 'end', value: number, durationMs: number, format: 'mp4' | 'gif'): RecordingTrimRange {
  if (!Number.isFinite(value)) return { ...range }
  const quantum = format === 'gif' ? 10 : 1
  const duration = Math.floor(durationMs / quantum) * quantum
  const ms = Math.round(value / quantum) * quantum
  return edge === 'start'
    ? { ...range, startMs: Math.max(0, Math.min(range.endMs - 20, ms)) }
    : { ...range, endMs: Math.max(range.startMs + 20, Math.min(duration, ms)) }
}
/** 根据录制帧率定位相邻帧时间；边界钳制，避免越过片段或出现浮点累计漂移。 */
export function recordingFrameTime(current: number, direction: -1 | 1, fps: number, range: RecordingTrimRange): number {
  const rate = Number.isFinite(fps) && fps > 0 && fps <= 240 ? fps : 30
  return Math.max(range.startMs, Math.min(range.endMs, Math.round((Math.round(current * rate / 1000) + direction) * 1000 / rate)))
}
export interface RecordingTrimHistory { past: RecordingTrimRange[]; present: RecordingTrimRange; future: RecordingTrimRange[] }
export function commitRecordingTrim(history: RecordingTrimHistory, range: RecordingTrimRange): RecordingTrimHistory {
  if (history.present.startMs === range.startMs && history.present.endMs === range.endMs) return history
  return { past: [...history.past, history.present].slice(-50), present: { ...range }, future: [] }
}
export function travelRecordingTrim(history: RecordingTrimHistory, redo = false): RecordingTrimHistory {
  const source = redo ? history.future : history.past, target = source.at(-1)
  if (!target) return history
  return redo ? { past: [...history.past, history.present], present: target, future: source.slice(0, -1) }
    : { past: source.slice(0, -1), present: target, future: [...history.future, history.present] }
}

export function recordingTrimRange(start: string, end: string, durationMs: number, format: 'mp4' | 'gif'): RecordingTrimRange {
  const parse = (value: string) => {
    if (!/^\d+(?:\.\d{1,3})?$/.test(value.trim())) throw new Error('请输入有效秒数，最多三位小数。')
    const milliseconds = Math.round(Number(value) * 1000)
    if (!Number.isSafeInteger(milliseconds)) throw new Error('时间超出有效范围。')
    return milliseconds
  }
  const startMs = parse(start), endMs = parse(end)
  if (!Number.isSafeInteger(durationMs) || durationMs < 20 || startMs >= endMs || endMs > durationMs || endMs - startMs < 20) {
    throw new Error('结束时间须大于开始时间，至少保留 0.02 秒且不能超过原片时长。')
  }
  if (format === 'gif' && (startMs % 10 !== 0 || endMs % 10 !== 0)) throw new Error('GIF 时间须精确到 0.01 秒。')
  return { startMs, endMs }
}
export function recordingClipTime(milliseconds: number): string {
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return '—'
  const value = Math.round(milliseconds)
  return `${Math.floor(value / 3600000).toString().padStart(2, '0')}:${Math.floor(value / 60000 % 60).toString().padStart(2, '0')}:${Math.floor(value / 1000 % 60).toString().padStart(2, '0')}.${(value % 1000).toString().padStart(3, '0')}`
}
