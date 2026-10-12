import { describe, expect, it } from 'vitest'
import { defaultSettings, exportFilenamePreview, validateExportSettings } from '../src/lib/settings'

describe('导出设置', () => {
  it('草稿与恢复默认的嵌套导出对象互不影响', () => {
    const draft = defaultSettings()
    draft.export.format = 'jpeg'
    draft.export.filenamePrefix = '我的截图'
    expect(defaultSettings().export.format).toBe('png')
    expect(defaultSettings().export.filenamePrefix).toBe('SnapDesk')
  })
  it('预览对齐格式扩展名与可选尺寸', () => {
    const settings = defaultSettings().export
    expect(exportFilenamePreview(settings)).toBe('SnapDesk-时间戳.png')
    settings.format = 'jpeg'; settings.includeDimensions = true
    expect(exportFilenamePreview(settings)).toBe('SnapDesk-时间戳-1920x1080.jpg')
    settings.format = 'webp'
    expect(exportFilenamePreview(settings)).toBe('SnapDesk-时间戳-1920x1080.webp')
  })
  it('拒绝路径、控制字符、空白或过长前缀', () => {
    for (const filenamePrefix of ['', '../escape', 'a\\b', 'a:b', ' a', 'a ', '.', '..', 'a\n', 'a\u0085b', '字'.repeat(49)]) {
      expect(validateExportSettings({ ...defaultSettings().export, filenamePrefix })).not.toBeNull()
    }
    for (const filenamePrefix of ['工作截图', '字'.repeat(48), '😀'.repeat(48)]) {
      expect(validateExportSettings({ ...defaultSettings().export, filenamePrefix })).toBeNull()
    }
  })
  it('质量只接受 1～100 整数，包括隐藏控件中的无效草稿', () => {
    for (const jpegQuality of [0, 101, 1.5, NaN, Infinity, '' as unknown as number]) {
      expect(validateExportSettings({ ...defaultSettings().export, jpegQuality })).not.toBeNull()
    }
    for (const jpegQuality of [1, 90, 100]) {
      expect(validateExportSettings({ ...defaultSettings().export, jpegQuality })).toBeNull()
    }
  })
})
