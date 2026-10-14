import { describe, expect, it } from 'vitest'
import { defaultSettings, exportFilenamePreview, validateExportSettings, validateShortcuts, captureShortcut, needsOnboarding } from '../src/lib/settings'

describe('导出设置', () => {
  it('草稿与恢复默认的嵌套导出对象互不影响', () => {
    const draft = defaultSettings()
    expect(draft.shortcutsEnabled).toBe(true)
    draft.shortcutsEnabled = false
    expect(defaultSettings().shortcutsEnabled).toBe(true)
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


describe('全局快捷键动作映射与录入', () => {
  const event = (key: string, code = key, modifiers = {}) => ({ key, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...modifiers })
  it('三个默认值独立复制，可分别清空并允许重新分配 F2/F3', () => {
    const shortcuts = defaultSettings().shortcuts
    expect(shortcuts).toEqual({ capture: 'F1', pin: 'F2', record: 'F3' })
    shortcuts.capture = 'F2'; shortcuts.pin = ''; shortcuts.record = ''
    expect(validateShortcuts(shortcuts)).toBeNull()
    expect(defaultSettings().shortcuts.capture).toBe('F1')
    expect(validateShortcuts({ capture: '', pin: '', record: '' })).toBeNull()
  })
  it('报告冲突动作，修饰键顺序不影响冲突判断', () => {
    const error = validateShortcuts({ capture: 'Ctrl+Shift+S', pin: 'Shift+Ctrl+S', record: '' })
    expect(error).toContain('区域截图'); expect(error).toContain('剪贴板图片贴图')
  })
  it('修饰键只预览，完整组合与功能键完成录入', () => {
    expect(captureShortcut(event('Control', 'ControlLeft', { ctrlKey: true }))).toEqual({ preview: 'Ctrl' })
    expect(captureShortcut(event('S', 'KeyS', { ctrlKey: true, shiftKey: true }))).toEqual({ value: 'Ctrl+Shift+S' })
    expect(captureShortcut(event('!', 'Digit1', { shiftKey: true }))).toEqual({ value: 'Shift+1' })
    for (let n = 1; n <= 12; n++) expect(captureShortcut(event(`F${n}`))).toEqual({ value: `F${n}` })
  })
  it('取消与清除有不同语义，拒绝普通字母和不支持的主键', () => {
    expect(captureShortcut(event('Escape'))).toEqual({ cancel: true })
    for (const key of ['Backspace', 'Delete']) expect(captureShortcut(event(key))).toEqual({ value: '' })
    for (const key of ['a', 'F', '1', 'F13', 'Enter']) expect(captureShortcut(event(key))).toHaveProperty('error')
    expect(captureShortcut(event('a', 'KeyA')).error).toContain('修饰键')
    expect(captureShortcut(event('f', 'KeyF')).error).toContain('修饰键')
    expect(validateShortcuts({ capture: 'F', pin: '', record: '' })).toContain('修饰键')
    expect(validateShortcuts({ capture: 'Ctrl+Ctrl+S', pin: '', record: '' })).toContain('重复')
  })
})


describe('独立引导版本', () => {
  it('仅首次或引导大版本变化时出现，未来版本不会倒退', () => {
    expect(needsOnboarding(defaultSettings().onboardingVersion)).toBe(true)
    expect(needsOnboarding(1)).toBe(false)
    expect(needsOnboarding(2)).toBe(false)
  })
})
