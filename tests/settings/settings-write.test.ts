import { afterEach, describe, expect, it, vi } from 'vitest'
import { defaultSettings } from '../../src/features/settings/settings'
const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
import { updateSettings } from '../../src/features/settings/settings-write'

afterEach(() => vi.resetAllMocks())
describe('跨页面自动保存队列', () => {
  it('截图与录屏连续修改时，以最新设置合并各自字段', async () => {
    let settings = defaultSettings()
    api.status.mockImplementation(async () => ({ settings: structuredClone(settings) }))
    api.saveSettings.mockImplementation(async value => { settings = structuredClone(value); return structuredClone(settings) })
    await Promise.all([
      updateSettings(current => ({ ...current, export: { ...current.export, filenamePrefix: 'Capture' } })),
      updateSettings(current => ({ ...current, recordingExport: { ...current.recordingExport, filenamePrefix: 'Video' } })),
    ])
    expect(settings.export.filenamePrefix).toBe('Capture')
    expect(settings.recordingExport.filenamePrefix).toBe('Video')
  })
  it('上一项失败后仍可保存其他设置', async () => {
    api.status.mockResolvedValue({ settings: defaultSettings() })
    api.saveSettings.mockRejectedValueOnce(new Error('配置写入失败')).mockImplementation(async value => value)
    await expect(updateSettings(current => current)).rejects.toThrow('配置写入失败')
    await expect(updateSettings(current => ({ ...current, delaySeconds: 3 }))).resolves.toMatchObject({ delaySeconds: 3 })
  })
})
