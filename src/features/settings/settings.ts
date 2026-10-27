import { t } from '@/shared/i18n/i18n'
import type { Appearance } from '@/shared/ui/appearance'
import { defaultAiSettings, type AiSettings } from '@/features/recognition/ai'
import { defaultClipboardSettings, type ClipboardSettings } from '@/features/clipboard/clipboard'
/** 与 Rust Settings / ExportSettings 的 camelCase 序列化契约一致。 */
export interface ExportSettings {
  format: 'png' | 'jpeg' | 'webp'
  jpegQuality: number
  filenamePrefix: string
  includeDimensions: boolean
  rememberDirectory: boolean
}
export type ShortcutAction = 'capture' | 'pin' | 'record' | 'history'
/** 空字符串表示单独禁用动作；总开关保留映射。 */
export type Shortcuts = Record<ShortcutAction, string>
export const shortcutActions: { key: ShortcutAction; label: string; description: string }[] = [
  { key: 'capture', label: '区域截图', description: '快速框选与标注' },
  { key: 'pin', label: '剪贴板图片贴图', description: '把剪贴板图片贴在桌面' },
  { key: 'record', label: '开始 / 停止录屏', description: '选择录制范围，录制中再次按下停止' },
  { key: 'history', label: '剪贴板历史', description: '打开历史管理页面' },
]
export const ONBOARDING_VERSION = 1
export function needsOnboarding(version: number): boolean { return version < ONBOARDING_VERSION }
export interface Settings { appearance: Appearance; ai: AiSettings; onboardingVersion: number; shortcuts: Shortcuts; shortcutsEnabled: boolean; delaySeconds: 0 | 3 | 5; screenshotCursor: boolean; screenshotSnap: boolean; export: ExportSettings; clipboard: ClipboardSettings }

/** 每次返回独立对象，避免编辑草稿时改动“恢复默认”的值。 */
export function defaultSettings(): Settings {
  return { appearance: { theme: 'system', language: 'zh-CN' }, ai: defaultAiSettings(), clipboard: defaultClipboardSettings(), onboardingVersion: 0, shortcuts: { capture: 'F1', pin: 'F2', record: 'F3', history: 'F4' }, shortcutsEnabled: true, delaySeconds: 0, screenshotCursor: false, screenshotSnap: true, export: {
    format: 'png', jpegQuality: 90, filenamePrefix: 'SnapDesk', includeDimensions: false, rememberDirectory: true,
  } }
}

export function exportFilenamePreview(value: ExportSettings): string {
  const dimensions = value.includeDimensions ? '-1920x1080' : ''
  return t('{0}-时间戳{1}.{2}', { 0: value.filenamePrefix, 1: dimensions, 2: value.format === 'jpeg' ? 'jpg' : value.format })
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
    if (!/^(?:[A-Z0-9]|F[1-9]|F1[0-2])$/.test(main)) return `${label}：主键仅支持字母、数字或 F1～F12。`
    if (!parts.length && !/^F(?:[1-9]|1[0-2])$/.test(main)) return `${label}：字母和数字必须包含修饰键。`
    if (new Set(parts).size !== parts.length || parts.some(p => !['Ctrl', 'Alt', 'Shift', 'Super'].includes(p))) return `${label}：修饰键无效或重复。`
    const normalized = normalizeShortcut(value)
    if (used.has(normalized)) return `${label}与${used.get(normalized)}的快捷键冲突。`
    used.set(normalized, label)
  }
  return null
}

/** 使用物理字母和数字键，避免 Shift 改变产生的字符。 */
export function captureShortcut(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): { value?: string; preview?: string; error?: string; cancel?: boolean } {
  if (event.key === 'Escape') return { cancel: true }
  if (event.key === 'Backspace' || event.key === 'Delete') return { value: '' }
  const modifiers = [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super'].filter(Boolean) as string[]
  if (['Control', 'Alt', 'Shift', 'Meta'].includes(event.key)) return { preview: modifiers.join(' + ') }
  const key = /^(Key[A-Z]|Digit[0-9])$/.test(event.code) ? event.code.replace(/^(Key|Digit)/, '') : event.key
  if (!/^(?:[A-Z0-9]|F[1-9]|F1[0-2])$/.test(key)) return { error: '主键仅支持字母、数字或 F1～F12。' }
  if (!modifiers.length && !/^F(?:[1-9]|1[0-2])$/.test(key)) return { error: '字母和数字必须包含修饰键。' }
  return { value: [...modifiers, key].join('+') }
}
