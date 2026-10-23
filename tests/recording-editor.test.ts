import { beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { recordingClipTime, recordingTrimRange } from '../src/lib/recording-editor'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'
import { ipcCommands } from '../src/lib/ipc-commands'
beforeEach(() => vi.resetAllMocks())

describe('录屏裁剪时间范围', () => {
  it('MP4 使用整数毫秒，允许整段及最小片段', () => {
    expect(recordingTrimRange('1.001', '5.000', 10000, 'mp4')).toEqual({ startMs: 1001, endMs: 5000 })
    expect(recordingTrimRange('0', '10', 10000, 'mp4')).toEqual({ startMs: 0, endMs: 10000 })
    expect(recordingTrimRange('0.001', '0.021', 1000, 'mp4')).toEqual({ startMs: 1, endMs: 21 })
  })
  it('GIF 要求百分之一秒，不能静默舍入用户选择', () => {
    expect(recordingTrimRange('0.05', '0.25', 300, 'gif')).toEqual({ startMs: 50, endMs: 250 })
    expect(() => recordingTrimRange('0.001', '0.1', 300, 'gif')).toThrow('0.01')
  })
  it.each(['', ' ', '-1', 'Infinity', 'NaN', '1e3', '1.0001', 'abc'])('无效开始时间不归一化为 0：%s', start => {
    expect(() => recordingTrimRange(start, '10', 10000, 'mp4')).toThrow()
  })
  it.each([['10', '10'], ['11', '10'], ['0', '10.001'], ['0', '0.019']])('拒绝越界、倒序及过短片段 %s～%s', (start, end) => {
    expect(() => recordingTrimRange(start, end, 10000, 'mp4')).toThrow('结束时间')
  })
  it('非法原片时长或溢出时间不发送后端', () => {
    for (const duration of [NaN, Infinity, -1, 19, 1000.5]) expect(() => recordingTrimRange('0', '1', duration, 'mp4')).toThrow()
    expect(() => recordingTrimRange('9007199254740991', '9007199254740992', 10000, 'mp4')).toThrow('时间超出')
  })
  it('长录屏时钟不绕回小时，显示毫秒', () => {
    expect(recordingClipTime(0)).toBe('00:00:00.000')
    expect(recordingClipTime(3661001)).toBe('01:01:01.001')
    expect(recordingClipTime(25 * 3600000)).toBe('25:00:00.000')
    expect(recordingClipTime(NaN)).toBe('—')
  })
})

describe('录屏编辑 IPC 和源文件归属', () => {
  it('按最近结果 ID 打开，返回原生时长及受限预览 token', async () => {
    const view = { token: 7, format: 'mp4', durationMs: 10000, previewAvailable: true }
    invoke.mockResolvedValueOnce(view)
    expect(await api.prepareRecordingEditor(42)).toEqual(view)
    expect(invoke).toHaveBeenLastCalledWith('prepare_recording_editor', { id: 42 })
  })
  it('另存只发送 token 和范围；取消选择返回 null，不传任意源/目标路径', async () => {
    const range = recordingTrimRange('1', '5', 10000, 'mp4')
    invoke.mockResolvedValueOnce(null)
    expect(await api.trimRecording(7, range)).toBeNull()
    expect(invoke).toHaveBeenLastCalledWith('trim_recording', { token: 7, range: { startMs: 1000, endMs: 5000 } })
    const output = { path: 'D:\\Videos\\clip-trimmed.mp4', durationMs: 4000, previewAvailable: true }
    invoke.mockResolvedValueOnce(output)
    expect(await api.trimRecording(7, range)).toEqual(output)
  })
  it('取消和关闭仅作用于本次编辑 token', async () => {
    await api.cancelRecordingTrim(7)
    expect(invoke).toHaveBeenLastCalledWith('cancel_recording_trim', { token: 7 })
    await api.closeRecordingEditor(7)
    expect(invoke).toHaveBeenLastCalledWith('close_recording_editor', { token: 7 })
  })
  it('失败可见，重试仍按同一录屏来源', async () => {
    invoke.mockRejectedValueOnce(new Error('录屏文件已被修改'))
    await expect(api.prepareRecordingEditor(42)).rejects.toThrow('录屏文件已被修改')
    invoke.mockResolvedValueOnce({ token: 8, format: 'gif', durationMs: 300, previewAvailable: true })
    expect((await api.prepareRecordingEditor(42)).token).toBe(8)
  })
  it('新增命令、诊断白名单和原生注册同步', () => {
    const native = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    const catalog = readFileSync(new URL('../src-tauri/src/diagnostics/catalog.rs', import.meta.url), 'utf8')
    for (const command of ['prepare_recording_editor', 'close_recording_editor', 'trim_recording', 'cancel_recording_trim']) {
      expect(ipcCommands).toContain(command)
      expect(native).toContain(`::${command},`)
      expect(catalog).toContain(`"${command}",`)
    }
  })
})
