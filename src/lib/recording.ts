import type { Rect } from './selection'

export interface RecordingDirectory { path: string; isDefault: boolean }

/** 与 Rust RecorderConfig / RecorderCapabilities 保持一致；声音不发送隐式默认开启值。 */
export type EncodingPreference = 'auto' | 'software'
export const encoderLabels = { hardware: '硬件编码', software: '软件编码', system: '系统自动（类型未确认）' } as const
export interface RecorderConfig {
  source: 'primaryMonitor'; region: Rect; fps: 30 | 60; captureCursor: boolean;
  format: 'mp4'; systemAudio: boolean; microphone: boolean; countdownSeconds: 0 | 3 | 5; encoding: EncodingPreference
}
export interface RecorderCapabilities {
  available: boolean; reason: string | null; sources: 'primaryMonitor'[];
  frameRates: number[]; captureCursor: boolean; systemAudio: boolean; microphone: boolean; formats: 'mp4'[]
}
export function recordingConfig(region: Rect, fps: 30 | 60 = 30, captureCursor = true, countdownSeconds: 0 | 3 | 5 = 0, encoding: EncodingPreference = 'auto'): RecorderConfig {
  return { source: 'primaryMonitor', region: { ...region }, fps, captureCursor, format: 'mp4', systemAudio: false, microphone: false, countdownSeconds, encoding }
}
/** 与 Rust RecordingView / RecordingAction 保持一致，不使用前端计时推测视频长度。 */
export type RecordingPhase = 'ready' | 'countdown' | 'recording' | 'paused' | 'encoding' | 'completed' | 'cancelled' | 'failed'
export type RecordingAction = 'start' | 'pause' | 'resume' | 'stop' | 'cancel'
export interface RecordingView {
  id: number; revision: number; phase: RecordingPhase; seconds: number;
  encoderKind: keyof typeof encoderLabels; encoderFallback: string | null;
  frames: number; droppedFrames: number; targetFps: number; countdownRemaining: number; message: string; path: string | null
}
export function recordingActive(view: RecordingView | null): boolean {
  return !!view && ['ready', 'countdown', 'recording', 'paused', 'encoding'].includes(view.phase)
}
export const recordingLabels: Record<RecordingPhase, string> = {
  ready: '待开始', countdown: '倒计时', recording: '录制中', paused: '已暂停', encoding: '正在保存',
  completed: '已保存', cancelled: '已取消', failed: '录制失败',
}
/** 只展示 Rust 返回的倒计时，窗口后台节流或重新挂载时也不会自行提前开始。 */
export function recordingStatus(view: RecordingView): string {
  return view.phase === 'countdown' ? `${view.countdownRemaining} 秒后开始` : recordingLabels[view.phase]
}
/** 忽略旧查询，防止暂停/停止后被尚未返回的采集状态覆盖。 */
export function newerRecording(current: RecordingView | null, next: RecordingView | null): RecordingView | null {
  if (!next) return current
  if (current && (next.id < current.id || (next.id === current.id && next.revision < current.revision))) return current
  return next
}
export function recordingTime(seconds: number): string {
  const value = Math.max(0, Math.floor(seconds))
  return `${Math.floor(value / 60).toString().padStart(2, '0')}:${(value % 60).toString().padStart(2, '0')}`
}
