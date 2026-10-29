import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ desktop: true, write: vi.fn(), invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => mocks.desktop, invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/plugin-log', () => ({ debug: mocks.write, info: mocks.write, warn: mocks.write, error: mocks.write }))
import { appLog, installGlobalErrorLogging, logFrontendFailure, setLogWindowKind } from '../../src/services/logger'
import { api } from '../../src/services/api'
import { defaultAiSettings } from '../../src/features/recognition/ai'

beforeEach(() => {
  vi.resetAllMocks()
  mocks.desktop = true
  mocks.write.mockResolvedValue(undefined)
  setLogWindowKind('unknown')
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

  it('全局监听只安装一次，仅记录异常类别，不输出异常正文', () => {
    const target = new EventTarget()
    installGlobalErrorLogging(target)
    installGlobalErrorLogging(target)
    target.dispatchEvent(Object.assign(new Event('error'), { message: 'private path', error: new Error('token') }))
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: 'OCR text' }))
    expect(mocks.write.mock.calls).toEqual([
      ['event=frontend.uncaught_error module=frontend window=unknown error_kind=Error'],
      ['event=frontend.unhandled_rejection module=frontend window=unknown error_kind=non_error'],
    ])
  })

  it('区分窗口、运行时异常和 Promise 异常，只保留有效数字位置', () => {
    const target = new EventTarget()
    setLogWindowKind('recording-controls')
    installGlobalErrorLogging(target)
    const error = new TypeError('private text')
    target.dispatchEvent(Object.assign(new Event('error'), {
      error, message: 'sk-secret', filename: 'C:/Users/private.js', lineno: 42, colno: 7,
    }))
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: new RangeError('OCR text') }))
    expect(mocks.write.mock.calls).toEqual([
      ['event=frontend.uncaught_error module=frontend window=recording-controls error_kind=TypeError line=42 column=7'],
      ['event=frontend.unhandled_rejection module=frontend window=recording-controls error_kind=RangeError'],
    ])
  })

  it('自定义异常名称、窗口标签和额外字段不能绕过白名单', () => {
    setLogWindowKind('pin-private-file')
    const error = new Error('sk-secret')
    error.name = 'user@example.com'
    logFrontendFailure('vue.component_error', error)
    const unsafe = appLog as (...args: unknown[]) => void
    unsafe('error', 'frontend.uncaught_error', { kind: 'sk-secret', line: -1, column: Infinity, stack: 'private' })
    expect(mocks.write.mock.calls).toEqual([
      ['event=vue.component_error module=frontend window=unknown error_kind=unknown'],
      ['event=frontend.uncaught_error module=frontend window=unknown error_kind=unknown'],
    ])
  })

  it('异常类别读取失败仍记录 unknown，不让日志抛出新异常', () => {
    const error = new Error('private')
    Object.defineProperty(error, 'name', { get() { throw new Error('secret') } })
    expect(() => logFrontendFailure('frontend.mount_failed', error)).not.toThrow()
    expect(mocks.write).toHaveBeenCalledExactlyOnceWith('event=frontend.mount_failed module=frontend window=unknown error_kind=unknown')
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
