import { describe, expect, it, vi } from 'vitest'
import { errorMessage } from '../src/lib/errors'

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))
import { api, onError } from '../src/lib/api'

const encode = (text: string) => new TextEncoder().encode(text)

describe('桌面 IPC 错误解码', () => {
  it('保留二进制 UTF-8 正文、JSON 字符串及结构化错误', () => {
    expect(errorMessage(encode('权限不足').buffer)).toBe('权限不足')
    expect(errorMessage(encode('"missing Origin header"').buffer)).toBe('missing Origin header')
    expect(errorMessage(encode('{"message":"请求失败"}').buffer)).toBe('请求失败')
    expect(errorMessage(new Error('原生错误'))).toBe('原生错误')
    expect(errorMessage('普通错误')).toBe('普通错误')
  })
  it('只解码视图范围，处理空响应和循环对象', () => {
    const bytes = encode('prefix:error:suffix')
    expect(errorMessage(new DataView(bytes.buffer, 7, 5))).toBe('error')
    expect(errorMessage(new ArrayBuffer(0))).toContain('空错误响应')
    const cycle: { self?: unknown } = {}; cycle.self = cycle
    expect(() => errorMessage(cycle)).not.toThrow()
  })
  it('命令失败附带命令名，成功响应不做转换', async () => {
    mocks.invoke.mockRejectedValueOnce(encode('原生拒绝请求').buffer)
    await expect(api.status()).rejects.toThrow('app_status：原生拒绝请求')
    const result = { settings: { shortcuts: { capture: 'Alt+Shift+S', pin: 'F2', record: 'F3' } } }
    mocks.invoke.mockResolvedValueOnce(result)
    expect(await api.status()).toBe(result)
  })
  it('事件订阅失败保留事件名和底层错误正文', async () => {
    mocks.listen.mockRejectedValueOnce(encode('event.listen not allowed').buffer)
    await expect(onError(() => {})).rejects.toThrow('订阅 capture-error：event.listen not allowed')
  })
})
