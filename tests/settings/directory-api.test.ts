import { beforeEach, expect, it, vi } from 'vitest'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
vi.mock('../../src/services/ipc', () => ({ invoke, listen }))
import { settingsApi } from '../../src/features/settings/api'
import { onSettingsImported } from '../../src/features/settings/events'
import { defaultSettings } from '../../src/features/settings/settings'
beforeEach(() => vi.resetAllMocks())
it('选择截图目录与恢复默认只传 reset，取消保持 null', async () => {
  invoke.mockResolvedValueOnce(null)
  expect(await settingsApi.chooseCaptureDirectory()).toBeNull()
  expect(invoke).toHaveBeenLastCalledWith('choose_capture_directory', { reset: false })
  const reset = { kind: 'captures', path: null, error: null }
  invoke.mockResolvedValueOnce(reset)
  expect(await settingsApi.chooseCaptureDirectory(true)).toEqual(reset)
  expect(invoke).toHaveBeenLastCalledWith('choose_capture_directory', { reset: true })
})

it('配置导入不允许传路径或配置内容，取消返回 null', async () => {
  invoke.mockResolvedValueOnce(null)
  expect(await settingsApi.importSettings()).toBeNull()
  expect(invoke).toHaveBeenLastCalledWith('import_settings')
})

it('导入完成事件只把配置 payload 交给主界面，保留取消订阅函数', async () => {
  const off = vi.fn(), receive = vi.fn(), settings = defaultSettings()
  listen.mockResolvedValueOnce(off)
  expect(await onSettingsImported(receive)).toBe(off)
  expect(listen.mock.calls[0]![0]).toBe('settings-imported')
  listen.mock.calls[0]![1]({ event: 'settings-imported', id: 1, payload: settings })
  expect(receive).toHaveBeenCalledWith(settings)
})
