import { beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import type { OcrResult } from '../../src/features/recognition/ai'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../../src/services/api'
import { ipcCommands } from '../../src/services/ipc-commands'
const result: OcrResult = {
  requestId: 'test', provider: 'local-ppocr', imageWidth: 600, imageHeight: 400,
  fullText: '第一行\nSecond line\n第三行', elapsedMs: 1, warnings: [],
  lines: ['第三行', '第一行', 'Second line'].map((text, index) => ({
    id: ['c', 'a', 'b'][index]!, order: [2, 0, 1][index]!, text,
    polygon: null, confidence: null, language: null, paragraphId: null,
  })),
}
beforeEach(() => vi.resetAllMocks())
describe('已有图片识别 IPC', () => {
  it('使用本窗口图片，不提交任意路径或窗口标签', async () => {
    invoke.mockResolvedValueOnce(result)
    expect(await api.recognizePinText()).toEqual(result)
    expect(invoke).toHaveBeenLastCalledWith('recognize_pin_text', undefined)
    const qr = { id: 3, items: [{ text: 'plain text', url: null }] }
    invoke.mockResolvedValueOnce(qr)
    expect(await api.recognizePinQr()).toEqual(qr)
    expect(invoke).toHaveBeenLastCalledWith('recognize_pin_qr', undefined)
  })
  it.each(['open', 'copy'] as const)('二维码动作 %s 不提交任意链接', async action => {
    await api.pinQrAction(17, 0, action)
    expect(invoke).toHaveBeenLastCalledWith('pin_qr_action', { id: 17, index: 0, action })
  })
  it('失败向上传递，后续可重试', async () => {
    invoke.mockRejectedValueOnce(new Error('OCR 资源不可用'))
    await expect(api.recognizePinText()).rejects.toThrow('OCR 资源不可用')
    invoke.mockResolvedValueOnce(result)
    expect(await api.recognizePinText()).toEqual(result)
  })
  it('新命令同步原生注册、目录和诊断白名单', () => {
    const native = readFileSync(new URL('../../src-tauri/src/app/mod.rs', import.meta.url), 'utf8')
    const catalog = readFileSync(new URL('../../src-tauri/src/diagnostics/catalog.rs', import.meta.url), 'utf8')
    for (const command of ['recognize_pin_text', 'recognize_pin_qr', 'pin_qr_action']) {
      expect(ipcCommands).toContain(command)
      expect(native).toContain(`::${command},`)
      expect(catalog).toContain(`"${command}",`)
    }
  })
})
