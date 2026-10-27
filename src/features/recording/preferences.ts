import { defaultInputEffects, type InputEffects, type RecordingFormat, type RecordingFps, type EncodingPreference, type RecordingQuality } from './recording'

const KEY = 'snap-desk.recording-preferences.v1'
export interface RecordingPreferences {
  format: RecordingFormat; mp4Fps: RecordingFps; gifFps: RecordingFps
  encoding: EncodingPreference; quality: RecordingQuality; mbps: number | null
  countdown: 0 | 3 | 5; captureCursor: boolean; effects: InputEffects
}
export interface PreferenceStorage { getItem(key: string): string | null; setItem(key: string, value: string): void }
export function defaultRecordingPreferences(): RecordingPreferences {
  return { format: 'mp4', mp4Fps: 60, gifFps: 10, encoding: 'auto', quality: 'balanced', mbps: null, countdown: 0, captureCursor: true, effects: defaultInputEffects() }
}
const record = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
const integer = (value: unknown, min: number, max: number): value is number => typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max

/** 只保存样式偏好，不保存选区、窗口身份或音频开关；旧/损坏字段逐项回退。 */
export function normalizeRecordingPreferences(value: unknown): RecordingPreferences {
  const result = defaultRecordingPreferences(), source = record(value), effects = record(source.effects)
  if (source.format === 'mp4' || source.format === 'gif') result.format = source.format
  if (source.mp4Fps === 30 || source.mp4Fps === 60) result.mp4Fps = source.mp4Fps
  if (source.gifFps === 10 || source.gifFps === 20) result.gifFps = source.gifFps
  if (source.encoding === 'auto' || source.encoding === 'software') result.encoding = source.encoding
  if (source.quality === 'economy' || source.quality === 'balanced' || source.quality === 'high') result.quality = source.quality
  if (typeof source.mbps === 'number' && Number.isFinite(source.mbps) && source.mbps >= 1 && source.mbps <= 80) result.mbps = source.mbps
  if (source.countdown === 0 || source.countdown === 3 || source.countdown === 5) result.countdown = source.countdown
  if (typeof source.captureCursor === 'boolean') result.captureCursor = source.captureCursor
  for (const field of ['keyboard', 'clicks', 'movement', 'drag', 'wheel'] as const) if (typeof effects[field] === 'boolean') result.effects[field] = effects[field]
  if (integer(effects.size, 12, 64)) result.effects.size = effects.size
  if (integer(effects.durationMs, 200, 2000)) result.effects.durationMs = effects.durationMs
  if (effects.position === 'topLeft' || effects.position === 'bottomLeft' || effects.position === 'bottomRight') result.effects.position = effects.position
  for (const field of ['leftColor', 'rightColor', 'doubleColor'] as const) {
    const color = effects[field]
    if (Array.isArray(color) && color.length === 3 && color.every(channel => integer(channel, 0, 255))) result.effects[field] = [...color] as [number, number, number]
  }
  return result
}
function browserStorage(): PreferenceStorage | undefined {
  try { return typeof localStorage === 'undefined' ? undefined : localStorage } catch { return undefined }
}
export function readRecordingPreferences(storage = browserStorage()): RecordingPreferences {
  try {
    const saved = JSON.parse(storage?.getItem(KEY) ?? 'null')
    return saved?.version === 1 ? normalizeRecordingPreferences(saved.preferences) : defaultRecordingPreferences()
  } catch { return defaultRecordingPreferences() }
}
export function saveRecordingPreferences(value: RecordingPreferences, storage = browserStorage()): boolean {
  try {
    if (!storage) return false
    storage.setItem(KEY, JSON.stringify({ version: 1, preferences: normalizeRecordingPreferences(value) }))
    return true
  } catch { return false /* 缓存不可写不影响本次录制，也不会回写不可信字段。 */ }
}
