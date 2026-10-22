import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
import { api, onCaptureClosed } from '../src/lib/api'
import { ipcCommands } from '../src/lib/ipc-commands'

it('将原生会话关闭编号交给前端释放图片，并返回取消订阅函数', async () => {
  const off = vi.fn(), closed = vi.fn()
  listen.mockResolvedValueOnce(off)
  expect(await onCaptureClosed(closed)).toBe(off)
  expect(listen.mock.calls.at(-1)?.[0]).toBe('capture-closed')
  listen.mock.calls.at(-1)?.[1]({ payload: 42 })
  expect(closed).toHaveBeenCalledWith(42)
})

describe('原生窗口截图 IPC', () => {
  it('提交会话和候选序号，返回新的冻结画面，不向前端暴露 HWND', async () => {
    const frame = { id: 12, width: 4480, height: 1440, dataUrl: 'data:image/png;base64,new', targets: [] }
    invoke.mockResolvedValueOnce(frame)
    expect(await api.nativeWindow(12, 3)).toEqual(frame)
    expect(invoke).toHaveBeenLastCalledWith('capture_native_window', { id: 12, target: 3 })
  })
  it('前端命令、原生注册与导出日志白名单同步', () => {
    expect(ipcCommands).toContain('capture_native_window')
    expect(readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')).toContain('capture::capture_native_window,')
    expect(readFileSync(new URL('../src-tauri/src/diagnostics/catalog.rs', import.meta.url), 'utf8')).toContain('"capture_native_window",')
  })
})

describe('二维滚动截图 IPC', () => {
  it('部分保存冻结可信画布，普通完成和取消沿用同一控制命令', async () => {
    await api.controlScroll(12, false, true)
    expect(invoke).toHaveBeenLastCalledWith('control_scroll', { id: 12, cancel: false, confirmedOnly: true })
    await api.controlScroll(12)
    expect(invoke).toHaveBeenLastCalledWith('control_scroll', { id: 12, cancel: false, confirmedOnly: false })
    await api.controlScroll(12, true)
    expect(invoke).toHaveBeenLastCalledWith('control_scroll', { id: 12, cancel: true, confirmedOnly: false })
  })
  const rect = { x: 10, y: 20, width: 640, height: 480 }
  it('旧调用默认自动识别和白底，不开启原始帧存档', async () => {
    invoke.mockResolvedValueOnce({ id: 12 })
    await api.startScroll(12, rect, false)
    expect(invoke).toHaveBeenLastCalledWith('start_scroll', {
      id: 12, rect, saveFrames: false, mode: 'auto', background: 'white',
    })
  })
  it('横向与透明背景选项通过同一命令发送', async () => {
    invoke.mockResolvedValueOnce({ id: 13 })
    await api.startScroll(13, rect, false, 'horizontal', 'transparent')
    expect(invoke).toHaveBeenLastCalledWith('start_scroll', {
      id: 13, rect, saveFrames: false, mode: 'horizontal', background: 'transparent',
    })
  })
})
