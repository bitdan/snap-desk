/** 与 Rust Settings / ExportSettings 的 camelCase 序列化契约一致。 */
export interface ExportSettings {
  format: 'png' | 'jpeg' | 'webp'
  jpegQuality: number
  filenamePrefix: string
  includeDimensions: boolean
  rememberDirectory: boolean
}
export interface Settings { shortcut: string; shortcutsEnabled: boolean; delaySeconds: 0 | 3 | 5; export: ExportSettings }

/** 每次返回独立对象，避免编辑草稿时改动“恢复默认”的值。 */
export function defaultSettings(): Settings {
  return { shortcut: 'F1', shortcutsEnabled: true, delaySeconds: 0, export: {
    format: 'png', jpegQuality: 90, filenamePrefix: 'SnapDesk', includeDimensions: false, rememberDirectory: true,
  } }
}

export function exportFilenamePreview(value: ExportSettings): string {
  const dimensions = value.includeDimensions ? '-1920x1080' : ''
  return `${value.filenamePrefix}-时间戳${dimensions}.${value.format === 'jpeg' ? 'jpg' : value.format}`
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
