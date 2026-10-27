import { beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import type { OcrResult } from '../../src/features/recognition/ai'
import { ocrPolygon, selectedOcrText } from '../../src/features/pins/pin-recognition'
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
describe('贴图文字选择与坐标', () => {
  it('保持阅读顺序，忽略重复及旧行 ID，不改原结果', () => {
    expect(selectedOcrText(result, ['c', 'a', 'a', 'missing'])).toBe('第一行\n第三行')
    expect(result.lines.map(line => line.id)).toEqual(['c', 'a', 'b'])
  })
  it('清空选择不退回全文，空识别也可处理', () => {
    expect(selectedOcrText(result, [])).toBe('')
    expect(selectedOcrText({ ...result, lines: [] }, ['a'])).toBe('')
  })
  it('保留原图坐标，由 SVG 处理缩放和 DPI', () => {
    expect(ocrPolygon([{ x: 120, y: 40 }, { x: 240, y: 40 }, { x: 240, y: 80 }], 600, 400)).toBe('120,40 240,40 240,80')
  })
  it('裁剪越界坐标', () => {
    expect(ocrPolygon([{ x: -5, y: 10 }, { x: 700, y: 10 }, { x: 300, y: 450 }], 600, 400)).toBe('0,10 600,10 300,400')
  })
  it.each([null, [], [{ x: 0, y: 0 }], [{ x: NaN, y: 0 }, { x: 1, y: 1 }, { x: 3, y: 2 }]])('异常框不绘制，文本仍可选择 %j', points => {
    expect(ocrPolygon(points, 600, 400)).toBe('')
  })
})
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
