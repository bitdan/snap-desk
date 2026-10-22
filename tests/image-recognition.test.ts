import { beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'
import { ipcCommands } from '../src/lib/ipc-commands'
import type { Annotation } from '../src/lib/annotations'

beforeEach(() => vi.resetAllMocks())

// 与 capture.rs / qr.rs 的现有命令契约对齐，不调用尚未实现的图片识别入口。
describe('截图二维码 IPC', () => {
  it('识别请求保留截图会话、裁剪及标注，并返回识别结果', async () => {
    const rect = { x: 1, y: 2, width: 300, height: 200 }
    const annotations: Annotation[] = [{ kind: 'rectangle', start: { x: 10, y: 20 }, end: { x: 40, y: 60 }, color: '#ff0000', width: 2 }]
    const result = { id: 9, items: [{ text: 'https://example.com/qr', url: 'https://example.com/qr' }] }
    invoke.mockResolvedValueOnce(result)
    expect(await api.recognizeQr(9, rect, annotations)).toEqual(result)
    expect(invoke).toHaveBeenLastCalledWith('recognize_capture_qr', { id: 9, rect, annotations })
  })
  it.each(['open', 'copy'] as const)('通过会话 ID 和结果索引执行 %s，不提交任意链接', async action => {
    await api.qrAction(7, 2, action)
    expect(invoke).toHaveBeenLastCalledWith('qr_action', { id: 7, index: 2, action })
  })
  it('二维码命令同步前端目录、原生注册和诊断白名单', () => {
    const native = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    const catalog = readFileSync(new URL('../src-tauri/src/diagnostics/catalog.rs', import.meta.url), 'utf8')
    for (const command of ['recognize_capture_qr', 'qr_action']) {
      expect(ipcCommands).toContain(command)
      expect(native).toContain(`::${command},`)
      expect(catalog).toContain(`"${command}",`)
    }
  })
})
