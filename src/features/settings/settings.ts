import type { RecognitionService } from '@/features/recognition/services'
import { defaultPinAppearance } from '@/features/pins/pins'
import type { PinAppearance } from '@/features/pins/types'
import type { Appearance } from '@/shared/ui/appearance'
import { defaultOcrSettings, type OcrSettings } from '@/features/recognition/ocr'
import { defaultAiSettings, type AiSettings } from '@/features/recognition/ai'
import { defaultClipboardSettings, type ClipboardSettings } from '@/features/clipboard/clipboard'
import { macPlatform } from '@/shared/platform'
/** 与 Rust Settings / ExportSettings 的 camelCase 序列化契约一致。 */
export interface ExportSettings {
  format: 'png' | 'jpeg' | 'webp'
  jpegQuality: number
  filenamePrefix: string
  includeDimensions: boolean
  rememberDirectory: boolean
}
export interface RecordingExportSettings { filenamePrefix: string; includeDimensions: boolean; rememberDirectory: boolean }
export function defaultRecordingExportSettings(): RecordingExportSettings {
  return { filenamePrefix: 'SnapDesk', includeDimensions: false, rememberDirectory: true }
}
export function validateRecordingExportSettings(value: RecordingExportSettings): string | null {
  return validateExportSettings({ ...defaultSettings().export, ...value })
}

export type ShortcutAction = 'capture' | 'pin' | 'record' | 'history' | 'translate' | 'shelf' | 'ocr'
/** 空字符串表示单独禁用动作；总开关保留映射。 */
export type Shortcuts = Record<ShortcutAction, string>
export const shortcutActions: { key: ShortcutAction; label: string; description: string }[] = [
  { key: 'capture', label: '区域截图', description: '快速框选与标注' },
  { key: 'pin', label: '剪贴板贴图', description: '把剪贴板文字或图片贴在桌面' },
  { key: 'record', label: '开始 / 停止录屏', description: '选择录制范围，录制中再次按下停止' },
  { key: 'history', label: '剪贴板历史', description: '打开历史管理页面' },
  { key: 'shelf', label: '展开 / 收起暂存架', description: '显示已开启的顶部暂存架' },
  { key: 'ocr', label: '识别文字', description: '框选屏幕文字并显示识别结果' },
  { key: 'translate', label: '翻译', description: '打开翻译界面，有选中文字时自动复制并翻译' },
]
export const ONBOARDING_VERSION = 1
export function needsOnboarding(version: number): boolean { return version < ONBOARDING_VERSION }
/** 后台检查更新间隔的可选档位（分钟），与原生端 UPDATE_CHECK_MINUTES 保持一致。 */
/** 与 Rust settings::UpdateNetwork 一致：均可留空；镜像是放在 github.com 地址前的 https:// 前缀，代理只用于检查和下载更新。 */
export interface UpdateNetwork { mirror: string; proxy: string }
/** 与 Rust settings::update_mirror 的规则一致，返回错误文案或 null。 */
export function validateUpdateMirror(value: string): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    if (value.length <= 512 && !/\s/.test(value) && url.protocol === 'https:' && url.hostname && !url.username && !url.password && !url.search && !url.hash) return null
  } catch { /* 下方统一提示 */ }
  return '下载镜像需填写完整的 https:// 地址，如 https://mirror.example/，不能包含空格、查询参数或账号密码。'
}
/** 与 Rust settings::update_proxy 的规则一致：http、https、socks5、socks5h，SOCKS 必须写端口。 */
export function validateUpdateProxy(value: string): string | null {
  if (!value) return null
  const invalid = '代理地址格式应为 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080。'
  const scheme = /^([a-z0-9]+):\/\//i.exec(value)?.[1]?.toLowerCase()
  if (!scheme || !['http', 'https', 'socks5', 'socks5h'].includes(scheme) || value.length > 512 || /\s/.test(value)) return invalid
  try {
    // URL 不识别 socks 协议的端口语义，换成 http 解析主机、端口与路径。
    const url = new URL(value.replace(/^[a-z0-9]+:/i, 'http:'))
    const port = /:(\d+)(\/)?$/.test(value.replace(/^[a-z0-9]+:\/\/[^@/]*@/i, ''))
    if (!url.hostname || !['', '/'].includes(url.pathname) || url.search || url.hash || (scheme.startsWith('socks') && !port)) return invalid
    return null
  } catch { return invalid }
}
export const updateCheckIntervals = [
  { value: 30, label: '30 分钟' }, { value: 60, label: '1 小时' }, { value: 360, label: '6 小时' }, { value: 1440, label: '24 小时' },
]
export interface Settings { recognitionServices: RecognitionService[]; receivePrereleaseUpdates: boolean; updateCheckMinutes: number; updateNetwork: UpdateNetwork; appearance: Appearance; ai: AiSettings; ocr: OcrSettings; onboardingVersion: number; shortcuts: Shortcuts; shortcutsEnabled: boolean; delaySeconds: 0 | 3 | 5; screenshotCursor: boolean; screenshotSnap: boolean; screenshotReuseLast: boolean; screenshotAfter: 'none' | 'copy' | 'pin' | 'save'; export: ExportSettings; pinAppearance: PinAppearance; recordingExport: RecordingExportSettings; clipboard: ClipboardSettings }

/** 每次返回独立对象，避免编辑草稿时改动“恢复默认”的值。 */
/** 与 Rust `Shortcuts::for_platform` 一致，只在应用状态读取前兜底；“恢复默认”使用状态中的 defaultShortcuts。
 * 截图、贴图、录屏三平台都是 F1–F3；macOS 的 ⌥+字母会输入特殊字符，其余功能改用 ⌃⌥+字母。 */
export function defaultShortcuts(mac = macPlatform): Settings['shortcuts'] {
  const modifier = mac ? 'Ctrl+Alt+' : 'Alt+'
  return { capture: 'F1', pin: 'F2', record: 'F3', history: `${modifier}V`, translate: `${modifier}T`, shelf: `${modifier}Z`, ocr: `${modifier}C` }
}

export function defaultSettings(): Settings {
  return { recognitionServices: [], receivePrereleaseUpdates: false, updateCheckMinutes: 60, updateNetwork: { mirror: '', proxy: '' }, pinAppearance: defaultPinAppearance(), recordingExport: defaultRecordingExportSettings(), appearance: { theme: 'system', language: 'zh-CN' }, ai: defaultAiSettings(), ocr: defaultOcrSettings(), clipboard: defaultClipboardSettings(), onboardingVersion: 0, shortcuts: defaultShortcuts(), shortcutsEnabled: true, delaySeconds: 0, screenshotCursor: false, screenshotSnap: true, screenshotReuseLast: false, screenshotAfter: 'none', export: {
    format: 'png', jpegQuality: 90, filenamePrefix: 'SnapDesk', includeDimensions: false, rememberDirectory: true,
  } }
}

/** 与原生端一致的本地日期时间片段：YYYYMMDD-HHMMSS，可选毫秒。 */
export function filenameStamp(now = new Date(), millis = false): string {
  const pad = (value: number, size = 2) => String(value).padStart(size, '0')
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`
  return millis ? `${stamp}-${pad(now.getMilliseconds(), 3)}` : stamp
}
export function exportFilenamePreview(value: ExportSettings, now = new Date()): string {
  const dimensions = value.includeDimensions ? '-1920x1080' : ''
  return `${value.filenamePrefix}-${filenameStamp(now)}${dimensions}.${value.format === 'jpeg' ? 'jpg' : value.format}`
}
/** 录屏文件名另带毫秒与随机标识，避免同一秒内的多段录制重名。 */
export function recordingFilenamePreview(prefix: string, includeDimensions: boolean, format: string, now = new Date()): string {
  return `${prefix}-Recording-${filenameStamp(now, true)}-a1b2${includeDimensions ? '-1920x1080' : ''}.${format}`
}

/** 与 Rust 同时校验；number 输入清空后可能为字符串，不能把无效值传给 IPC。 */
export function validateExportSettings(value: ExportSettings): string | null {
  if (!['png', 'jpeg', 'webp'].includes(value.format)) return '请选择支持的图片格式。'
  if (!Number.isInteger(value.jpegQuality) || value.jpegQuality < 1 || value.jpegQuality > 100) return 'JPEG 质量必须为 1～100 的整数。'
  const prefix = value.filenamePrefix
  if (!prefix || [...prefix].length > 48 || prefix.trim() !== prefix || prefix === '.' || prefix === '..'
    || /[<>:"/\\|?*\p{Cc}]/u.test(prefix)) return '文件名前缀须为 1～48 个字符，不能包含路径、控制字符或 <>:"/\\|?*，首尾不能有空白。'
  return null
}

/** 与原生 global-hotkey 解析器的主键名称一致；符号使用物理键名，避免加号分隔和布局歧义。 */
const shortcutKeys = new Set([
  'Backquote', 'Backslash', 'BracketLeft', 'BracketRight', 'Comma', 'Equal', 'Minus', 'Period', 'Quote', 'Semicolon', 'Slash',
  'Backspace', 'CapsLock', 'Enter', 'Space', 'Tab', 'Delete', 'End', 'Home', 'Insert', 'PageDown', 'PageUp',
  'PrintScreen', 'ScrollLock', 'Pause', 'Escape', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'NumLock',
  'NumpadAdd', 'NumpadDecimal', 'NumpadDivide', 'NumpadEnter', 'NumpadEqual', 'NumpadMultiply', 'NumpadSubtract',
  'AudioVolumeDown', 'AudioVolumeUp', 'AudioVolumeMute', 'MediaPlay', 'MediaPause', 'MediaPlayPause', 'MediaStop', 'MediaTrackNext', 'MediaTrackPrevious',
])
function supportedShortcutKey(key: string): boolean {
  return /^(?:[A-Z0-9]|F(?:[1-9]|1[0-9]|2[0-4])|Numpad[0-9])$/.test(key) || shortcutKeys.has(key)
}

/** 修饰键顺序归一化，避免漏掉同一组合的冲突。 */
export function normalizeShortcut(value: string): string {
  const parts = value.split('+')
  const key = parts.pop()!
  return [...['Ctrl', 'Alt', 'Shift', 'Super'].filter(part => parts.includes(part)), key].join('+')
}
export function validateShortcuts(shortcuts: Shortcuts): string | null {
  const used = new Map<string, string>()
  for (const { key, label } of shortcutActions) {
    const value = shortcuts[key]
    if (!value) continue
    const parts = value.split('+'), main = parts.pop()!
    if (!supportedShortcutKey(main)) return `${label}：不支持此按键，请使用其他按键。`
    if (new Set(parts).size !== parts.length || parts.some(p => !['Ctrl', 'Alt', 'Shift', 'Super'].includes(p))) return `${label}：修饰键无效或重复。`
    const normalized = normalizeShortcut(value)
    if (used.has(normalized)) return `${label}与${used.get(normalized)}的快捷键冲突。`
    used.set(normalized, label)
  }
  return null
}

/** 使用物理字母和数字键，避免 Shift 改变产生的字符。 */
export function captureShortcut(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): { value?: string; preview?: string; error?: string; cancel?: boolean } {
  const modifiers = [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super'].filter(Boolean) as string[]
  if (!modifiers.length && event.key === 'Escape') return { cancel: true }
  if (!modifiers.length && (event.key === 'Backspace' || event.key === 'Delete')) return { value: '' }
  // macOS 的 Fn / 地球仪键只是辅助键：单独按下不作为主键，等待真正的主键。
  if (['Control', 'Alt', 'Shift', 'Meta', 'Fn', 'FnLock', 'Hyper', 'Super'].includes(event.key)) return { preview: modifiers.join(' + ') }
  const physical = /^(Key[A-Z]|Digit[0-9])$/.test(event.code) ? event.code.replace(/^(Key|Digit)/, '') : event.code
  const key = supportedShortcutKey(physical) ? physical : event.key === ' ' ? 'Space' : /^[a-z]$/.test(event.key) ? event.key.toUpperCase() : event.key
  if (!supportedShortcutKey(key)) return { error: '不支持此按键，请使用其他按键。' }
  return { value: [...modifiers, key].join('+') }
}

/**
 * 录入的组合是否已全部松开。macOS 的 WebKit 在按住 ⌘ 时不派发主键的 keyup，
 * 含 Super 的组合以 ⌘ 松开代替主键松开，否则录入会一直停在“请按快捷键…”。
 */
export function shortcutReleased(
  candidate: { value: string; released: boolean },
  event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>,
): boolean {
  if (event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return false
  return candidate.released || (event.key === 'Meta' && candidate.value.split('+').includes('Super'))
}
