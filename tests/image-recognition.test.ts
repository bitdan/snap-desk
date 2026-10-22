import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
const invoke = vi.hoisted(() => vi.fn().mockResolvedValue({ id: 1, links: false, items: [] }))
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'
import { ipcCommands } from '../src/lib/ipc-commands'

describe('图片二维码及文本链接 IPC', () => {
  it('普通截图区分二维码和 OCR 链接识别，保留裁剪及标注', async () => {
    const rect = { x: 1, y: 2, width: 300, height: 200 }
    await api.recognizeQr(9, rect, [])
    expect(invoke).toHaveBeenLastCalledWith('recognize_capture_qr', { id: 9, rect, annotations: [], links: false })
    await api.recognizeQr(9, rect, [], true)
    expect(invoke).toHaveBeenLastCalledWith('recognize_capture_qr', { id: 9, rect, annotations: [], links: true })
  })
  it('贴图由原生窗口身份选取图片，滚动截图携带会话 ID', async () => {
    await api.recognizePinQr(true)
    expect(invoke).toHaveBeenLastCalledWith('recognize_pin_qr', { links: true })
    await api.recognizeScrollQr(42)
    expect(invoke).toHaveBeenLastCalledWith('recognize_scroll_qr', { id: 42, links: false })
    await api.qrAction(7, 2, 'open')
    expect(invoke).toHaveBeenLastCalledWith('qr_action', { id: 7, index: 2, action: 'open' })
  })
  it('新入口同步原生注册和诊断白名单', () => {
    const native = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    const catalog = readFileSync(new URL('../src-tauri/src/diagnostics/catalog.rs', import.meta.url), 'utf8')
    for (const command of ['recognize_pin_qr', 'recognize_scroll_qr']) {
      expect(ipcCommands).toContain(command)
      expect(native).toContain(`::${command},`)
      expect(catalog).toContain(`"${command}",`)
    }
  })
})
