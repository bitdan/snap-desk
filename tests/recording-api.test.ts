import { describe, expect, it, vi } from 'vitest'
import { defaultInputEffects, recordingConfig } from '../src/lib/recording'
import { defaultSettings } from '../src/lib/settings'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'

describe('录屏 IPC 契约', () => {
  it('恢复导出仅传后台标识，不允许前端指定源文件路径', async () => {
    const items = [{ id: 'recording-pending-example', path: 'D:\\Videos\\video.mp4', finalized: true, stage: 'failed:syncing', exported: null }]
    invoke.mockResolvedValueOnce(items)
    expect(await api.recoverableRecordings()).toEqual(items)
    expect(invoke).toHaveBeenLastCalledWith('recoverable_recordings', undefined)
    invoke.mockResolvedValueOnce('D:\\Videos\\recovered.mp4')
    expect(await api.retryRecordingExport(items[0].id)).toBe('D:\\Videos\\recovered.mp4')
    expect(invoke).toHaveBeenLastCalledWith('retry_recording_export', { id: items[0].id })
  })
  it('快捷键总开关只传布尔值，不携带其他未保存设置', async () => {
    const settings = { ...defaultSettings(), shortcutsEnabled: false }
    invoke.mockResolvedValueOnce(settings)
    expect(await api.setShortcutsEnabled(false)).toEqual(settings)
    expect(invoke).toHaveBeenLastCalledWith('set_shortcuts_enabled', { enabled: false })
  })
  it('查询、选择和恢复录屏目录，取消原生选择器不产生新目录', async () => {
    const directory = { path: 'D:\\Videos\\录屏', isDefault: false }
    invoke.mockResolvedValueOnce(directory)
    expect(await api.currentRecordingDirectory()).toEqual(directory)
    expect(invoke).toHaveBeenLastCalledWith('current_recording_directory', undefined)
    invoke.mockResolvedValueOnce(null)
    expect(await api.chooseRecordingDirectory()).toBeNull()
    expect(invoke).toHaveBeenLastCalledWith('choose_recording_directory', { reset: false })
    const defaultDirectory = { path: 'C:\\AppData\\recordings', isDefault: true }
    invoke.mockResolvedValueOnce(defaultDirectory)
    expect(await api.chooseRecordingDirectory(true)).toEqual(defaultDirectory)
    expect(invoke).toHaveBeenLastCalledWith('choose_recording_directory', { reset: true })
  })
  it('保持原命令及会话 ID，同时发送配置和物理像素选区', async () => {
    const rect = { x: 13, y: 21, width: 101, height: 99 }
    const effects = { ...defaultInputEffects(), keyboard: true, wheel: true, drag: true }
    const config = recordingConfig(rect, 60, false, 5, 'software', 'high', effects, 8_500_000)
    expect(config.bitrate).toBe(8_500_000)
    expect(config.countdownSeconds).toBe(5)
    expect(config.encoding).toBe('software')
    expect(config.quality).toBe('high')
    expect(config.inputEffects).toEqual(effects)
    invoke.mockResolvedValueOnce(undefined)
    await api.startRecording(42, rect, config)
    expect(invoke).toHaveBeenLastCalledWith('start_recording', { id: 42, rect, config })
    await api.controlRecording(42, 'stop')
    expect(invoke).toHaveBeenLastCalledWith('control_recording', { id: 42, action: 'stop' })
  })

  it('保留平台不支持原因，不在前端假定 Windows 能力', async () => {
    const capabilities = { available: false, reason: 'UnsupportedPlatform：macOS / Linux 原生录屏尚未实现。', sources: [], frameRates: [], captureCursor: false, systemAudio: false, microphone: false, formats: [] }
    invoke.mockResolvedValueOnce(capabilities)
    expect(await api.recordingCapabilities()).toBe(capabilities)
    expect(invoke).toHaveBeenLastCalledWith('recording_capabilities', undefined)
  })
})
