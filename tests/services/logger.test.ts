import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ desktop: true, write: vi.fn(), invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => mocks.desktop, invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/plugin-log', () => ({ debug: mocks.write, info: mocks.write, warn: mocks.write, error: mocks.write }))
import { appLog, installGlobalErrorLogging } from '../../src/services/logger'
import { api } from '../../src/services/api'
import { defaultAiSettings } from '../../src/features/recognition/ai'

beforeEach(() => {
  vi.resetAllMocks()
  mocks.desktop = true
  mocks.write.mockResolvedValue(undefined)
})

describe('日志隐私与异常隔离', () => {
  it('IPC 失败仅记录固定命令，保留 UI 错误但不泄漏参数和异常正文', async () => {
    const secret = 'Bearer sk-secret user@example.com C:\\Users\\private OCR正文 剪贴板正文'
    mocks.invoke.mockRejectedValueOnce(new Error(secret))
    await expect(api.translateText('private-id', secret, true, defaultAiSettings())).rejects.toThrow(secret)
    expect(mocks.write).toHaveBeenCalledExactlyOnceWith('event=ipc.failed module=frontend command=translate_text code=IPC_FAILED')
  })

  it('忽略精确取消和正常保存对话框取消，不把真正错误误判为取消', async () => {
    mocks.invoke.mockRejectedValueOnce('翻译已取消。')
    await expect(api.translateText('id', 'text', true, defaultAiSettings())).rejects.toThrow('翻译已取消。')
    mocks.invoke.mockResolvedValueOnce(false)
    expect(await api.exportSettings()).toBe(false)
    expect(mocks.write).not.toHaveBeenCalled()
    mocks.invoke.mockRejectedValueOnce('无法取消：存储故障')
    await expect(api.cancelAiJob('id')).rejects.toThrow('存储故障')
    expect(mocks.write).toHaveBeenCalledTimes(1)
  })

  it('运行时拒绝未知事件、未知命令，丢弃多余敏感字段', () => {
    // 模拟未经过 TypeScript 的调用，验证白名单同样在运行时生效。
    const unsafe = appLog as (...args: unknown[]) => void
    unsafe('error', 'user@example.com')
    unsafe('error', 'ipc.failed', { command: 'sk-secret' })
    unsafe('error', 'ipc.failed', { command: 'app_status', token: 'sk-secret' })
    expect(mocks.write).toHaveBeenCalledExactlyOnceWith('event=ipc.failed module=frontend command=app_status code=IPC_FAILED')
  })

  it('日志失败不覆盖业务异常、不产生未处理拒绝，浏览器预览不调用插件', async () => {
    mocks.write.mockRejectedValueOnce(new Error('log unavailable'))
    mocks.invoke.mockRejectedValueOnce('original failure')
    await expect(api.status()).rejects.toThrow('original failure')
    await Promise.resolve()
    mocks.write.mockImplementationOnce(() => { throw new Error('sync failure') })
    expect(() => appLog('error', 'frontend.mount_failed')).not.toThrow()
    mocks.desktop = false
    appLog('error', 'frontend.mount_failed')
    expect(mocks.write).toHaveBeenCalledTimes(2)
  })

  it('全局监听只安装一次，异常对象完全不读入日志', () => {
    const target = new EventTarget()
    installGlobalErrorLogging(target)
    installGlobalErrorLogging(target)
    target.dispatchEvent(Object.assign(new Event('error'), { message: 'private path', error: new Error('token') }))
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: 'OCR text' }))
    expect(mocks.write.mock.calls).toEqual([
      ['event=frontend.uncaught_error module=frontend'],
      ['event=frontend.unhandled_rejection module=frontend'],
    ])
  })

  it('日志目录沿用固定种类 IPC', async () => {
    await api.openAppDirectory('logs')
    expect(mocks.invoke).toHaveBeenLastCalledWith('open_app_directory', { kind: 'logs' })
    await api.copyAppDirectory('logs')
    expect(mocks.invoke).toHaveBeenLastCalledWith('copy_app_directory', { kind: 'logs' })
  })

  it('跨窗口错误只关联白名单命令的数值会话，不记录用户提供的字符串 ID', async () => {
    mocks.invoke.mockRejectedValueOnce('encoder failed')
    await expect(api.controlRecording(42, 'stop')).rejects.toThrow('encoder failed')
    expect(mocks.write).toHaveBeenLastCalledWith('event=ipc.failed module=frontend command=control_recording code=IPC_FAILED op=rec-42')
    const unsafe = appLog as (...args: unknown[]) => void
    unsafe('error', 'ipc.failed', { command: 'control_recording', session: 'private-name' })
    expect(mocks.write).toHaveBeenLastCalledWith('event=ipc.failed module=frontend command=control_recording code=IPC_FAILED')
    unsafe('error', 'ipc.failed', { command: 'clipboard_restore', session: 123 })
    expect(mocks.write).toHaveBeenLastCalledWith('event=ipc.failed module=frontend command=clipboard_restore code=IPC_FAILED')
  })
})
