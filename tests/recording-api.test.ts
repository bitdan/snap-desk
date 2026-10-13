import { describe, expect, it, vi } from 'vitest'
import { recordingConfig } from '../src/lib/recording'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'

describe('录屏 IPC 契约', () => {
  it('保持原命令及会话 ID，同时发送配置和物理像素选区', async () => {
    const rect = { x: 13, y: 21, width: 101, height: 99 }
    const config = recordingConfig(rect, 60, false)
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
