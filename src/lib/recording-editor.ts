/** 与原生录屏编辑器契约一致：毫秒整数，路径只展示，所有动作按原生 token 执行。 */
export interface RecordingEditorView { token: number; format: 'mp4' | 'gif'; durationMs: number; previewAvailable: boolean }
export interface RecordingTrimRange { startMs: number; endMs: number }
export interface RecordingTrimExport { path: string; durationMs: number; previewAvailable: boolean }

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
