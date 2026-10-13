import type { Rect } from './selection'

/** 与 Rust RecorderConfig / RecorderCapabilities 保持一致；声音不发送隐式默认开启值。 */
export interface RecorderConfig {
  source: 'primaryMonitor'; region: Rect; fps: 30 | 60; captureCursor: boolean;
  format: 'mp4'; systemAudio: boolean; microphone: boolean
}
export interface RecorderCapabilities {
  available: boolean; reason: string | null; sources: 'primaryMonitor'[];
  frameRates: number[]; captureCursor: boolean; systemAudio: boolean; microphone: boolean; formats: 'mp4'[]
}
export function recordingConfig(region: Rect, fps: 30 | 60 = 30, captureCursor = true): RecorderConfig {
  return { source: 'primaryMonitor', region: { ...region }, fps, captureCursor, format: 'mp4', systemAudio: false, microphone: false }
}
/** 与 Rust RecordingView / RecordingAction 保持一致，不使用前端计时推测视频长度。 */
export type RecordingPhase = 'ready' | 'recording' | 'paused' | 'encoding' | 'completed' | 'cancelled' | 'failed'
export type RecordingAction = 'start' | 'pause' | 'resume' | 'stop' | 'cancel'
export interface RecordingView {
  id: number; revision: number; phase: RecordingPhase; seconds: number;
  frames: number; droppedFrames: number; targetFps: number; message: string; path: string | null
}
export function recordingActive(view: RecordingView | null): boolean {
  return !!view && ['ready', 'recording', 'paused', 'encoding'].includes(view.phase)
}
export const recordingLabels: Record<RecordingPhase, string> = {
  ready: '待开始', recording: '录制中', paused: '已暂停', encoding: '正在保存',
  completed: '已保存', cancelled: '已取消', failed: '录制失败',
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
