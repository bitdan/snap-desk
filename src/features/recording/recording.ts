import { t } from '@/shared/i18n/i18n'
import type { Rect } from '@/shared/geometry/selection'

export type RecordingSource = 'primaryMonitor' | 'selectedMonitor' | 'selectedWindow'
export type RecordingSelection = 'region' | 'display'

/** 输入/输出均是虚拟桌面图片坐标；负屏幕原点只参与平移，不按 DPI 缩放。 */
export function recordingRegion(region: Rect, mode: RecordingSelection, displays: { x: number; y: number; width: number; height: number }[], origin: { x: number; y: number }): Rect {
  const display = displays.find(d => region.x + origin.x >= d.x && region.y + origin.y >= d.y
    && region.x + origin.x + region.width <= d.x + d.width && region.y + origin.y + region.height <= d.y + d.height)
  if (!display) throw new Error('请选择单个显示器内的区域。')
  return mode === 'display' && display ? { x: display.x - origin.x, y: display.y - origin.y, width: display.width, height: display.height } : { ...region }
}

export interface RecordingDirectory { path: string; isDefault: boolean }
export interface RecoverableRecording { id: string; path: string; finalized: boolean; stage: string; exported: string | null; sourceExists: boolean }

export function recordingAudioStatus(view: Pick<RecordingView, 'systemAudio' | 'microphone'>): string {
  return [view.systemAudio && '系统声音', view.microphone && '麦克风'].filter((value): value is string => !!value).map(value => t(value)).join(' + ') || t('无声')
}

/** 与 Rust RecorderConfig / RecorderCapabilities 保持一致；声音不发送隐式默认开启值。 */
export type RecordingFormat = 'mp4' | 'gif'
export type RecordingFps = 10 | 20 | 30 | 60
export type RecordingQuality = 'economy' | 'balanced' | 'high'
export type EncodingPreference = 'auto' | 'software'
export const encoderLabels = { hardware: '硬件编码', software: '软件编码', system: '系统自动（类型未确认）' } as const
/** 与 Rust InputEffects 一致；默认关闭监听，不支持普通文字输入显示。 */
export interface InputEffects {
  keyboard: boolean; clicks: boolean; movement: boolean; drag: boolean; wheel: boolean;
  size: number; durationMs: number; position: 'topLeft' | 'bottomLeft' | 'bottomRight';
  leftColor: [number, number, number]; rightColor: [number, number, number]; doubleColor: [number, number, number]
}
export function defaultInputEffects(): InputEffects {
  return { keyboard: false, clicks: false, movement: false, drag: false, wheel: false,
    size: 20, durationMs: 400, position: 'topLeft',
    leftColor: [64, 160, 255], rightColor: [255, 112, 80], doubleColor: [190, 100, 255] }
}
export interface RecorderConfig {
  source: RecordingSource; region: Rect; fps: RecordingFps; captureCursor: boolean;
  format: RecordingFormat; systemAudio: boolean; microphone: boolean; countdownSeconds: 0 | 3 | 5; encoding: EncodingPreference; quality: RecordingQuality
  inputEffects: InputEffects
  /** 开始和继续时是否收起主界面；本次录制固定使用确认时的偏好。 */
  hideMainWindow: boolean
  /** bit/s；null 使用画质预设计算目标码率。 */
  bitrate: number | null
}
export interface RecorderCapabilities {
  available: boolean; reason: string | null; sources: RecordingSource[];
  frameRates: number[]; captureCursor: boolean; systemAudio: boolean; microphone: boolean; formats: RecordingFormat[]
}
/** 表单以 Mbps 输入，空白恢复自动；拒绝无效值，避免 NaN 经 IPC 变成 null。 */
export function recordingBitrate(value: string | number): number | null {
  if (typeof value === 'string' && !value.trim()) return null
  const mbps = Number(value)
  if (!Number.isFinite(mbps) || mbps < 1 || mbps > 80) throw new Error('自定义录屏码率需为 1–80 Mbps。')
  return Math.round(mbps * 1_000_000)
}
export function recordingConfig(region: Rect, fps: RecordingFps = 60, captureCursor = true, countdownSeconds: 0 | 3 | 5 = 0, encoding: EncodingPreference = 'auto', quality: RecordingQuality = 'balanced', effects: InputEffects = defaultInputEffects(), bitrate: number | null = null, format: RecordingFormat = 'mp4', systemAudio = false, microphone = false, hideMainWindow = true): RecorderConfig {
  if (!(format === 'gif' ? [10, 20] : [30, 60]).includes(fps)) throw new Error('帧率与录制格式不匹配。')
  // GIF 无音轨；从 MP4 切换后也不能把隐藏的声音偏好发给后端。
  if (format === 'gif') { bitrate = null; encoding = 'software'; systemAudio = false; microphone = false }
  if (bitrate !== null && (!Number.isInteger(bitrate) || bitrate < 1_000_000 || bitrate > 80_000_000)) throw new Error('自定义录屏码率需为 1–80 Mbps。')
  // 拷贝颜色数组，确认选区后改动表单不能改变已提交的配置快照。
  const inputEffects: InputEffects = { ...effects, leftColor: [...effects.leftColor], rightColor: [...effects.rightColor], doubleColor: [...effects.doubleColor] }
  return { source: 'primaryMonitor', region: { ...region }, fps, captureCursor, format, systemAudio, microphone, countdownSeconds, encoding, quality, inputEffects, bitrate, hideMainWindow }
}
/** 与 Rust RecordingView / RecordingAction 保持一致，不使用前端计时推测视频长度。 */
export type RecordingPhase = 'ready' | 'countdown' | 'recording' | 'paused' | 'encoding' | 'completed' | 'cancelled' | 'failed'
export type RecordingAction = 'start' | 'pause' | 'resume' | 'stop' | 'cancel'
/** 空格只控制录制状态，不抢占文字输入、输入法组词或内层已处理的按键。 */
export function recordingSpaceAction(event: KeyboardEvent, phase?: RecordingPhase): RecordingAction | null {
  if (event.key !== ' ' || event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229 || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return null
  if ((event.target as HTMLElement)?.closest?.('input, textarea, select, [contenteditable=true]')) return null
  return phase === 'ready' ? 'start' : phase === 'recording' ? 'pause' : phase === 'paused' ? 'resume' : null
}
/** 本次运行最近一次成功导出，包含恢复导出；路径只供展示，操作只传 ID。 */
export interface RecordingResult { id: number; path: string; filename: string }
export type RecordingResultAction = 'openDirectory' | 'copyPath'
/** 与 Rust RecordingStats 一致，耗时为累计毫秒；空取样不是丢帧。 */
export interface RecordingDiagnostics {
  capturePolls: number; capturedFrames: number; captureMillis: number;
  captureCopyBytes: number; compositionCopyBytes: number; nv12OutputBytes: number;
  throttledSamples: number; composedFrames: number; compositionMillis: number;
  queuedFrames: number; queueDroppedFrames: number; staleQueuedFrames: number;
  rejectedCaptureFrames: number; rejectedComposedFrames: number; recentFps: number
}
export interface RecordingView {
  systemAudio: boolean; microphone: boolean;
  id: number; revision: number; phase: RecordingPhase; seconds: number;
  encoderKind: keyof typeof encoderLabels; encoderFallback: string | null;
  encodingMillis: number; targetBitrate: number;
  stats: RecordingDiagnostics;
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

/** 全程平均及后端计算的近两秒有效 FPS；静态桌面数值下降不等同于丢帧。 */
export function recordingStats(view: RecordingView): string {
  const fps = view.seconds > 0 ? view.frames / view.seconds : 0
  return `平均 ${fps.toFixed(1)} FPS · 近 2 秒 ${view.stats.recentFps.toFixed(1)} FPS`
}
