import { describe, expect, it, vi } from 'vitest'
import { prepareRecordingSession } from '../../src/features/recording/use-recording-setup'

// 直接测试录屏接管协议，不再读取组件源码或依赖私有函数名称。
function fixture() {
  const calls: string[] = []
  const prepare = vi.fn(async () => { calls.push('prepare') })
  const activate = vi.fn(async () => { calls.push('activate') })
  const cancel = vi.fn(async () => { calls.push('cancel') })
  const onPrepared = vi.fn(() => { calls.push('release-image') })
  const start = () => prepareRecordingSession(prepare, activate, cancel, onPrepared)
  return { calls, prepare, activate, cancel, onPrepared, start }
}

describe('选区工具栏准备录制', () => {
  it('准备完成后才交出背景和激活原窗口，不隐式开始采集', async () => {
    const f = fixture()
    await f.start()
    expect(f.calls).toEqual(['prepare', 'release-image', 'activate'])
    expect(f.cancel).not.toHaveBeenCalled()
  })
  it('准备失败保留原选区，不尝试接管或取消不存在的会话', async () => {
    const f = fixture(), error = new Error('采集设备不可用')
    f.prepare.mockRejectedValueOnce(error)
    await expect(f.start()).rejects.toBe(error)
    expect(f.onPrepared).not.toHaveBeenCalled()
    expect(f.activate).not.toHaveBeenCalled()
    expect(f.cancel).not.toHaveBeenCalled()
  })
  it('接管失败取消已准备的会话，不遗留等待开始状态', async () => {
    const f = fixture(), error = new Error('窗口保护不可用')
    f.activate.mockRejectedValueOnce(error)
    await expect(f.start()).rejects.toBe(error)
    expect(f.cancel).toHaveBeenCalledOnce()
  })
  it('清理失败不覆盖最初的接管错误', async () => {
    const f = fixture(), error = new Error('窗口保护不可用')
    f.activate.mockRejectedValueOnce(error); f.cancel.mockRejectedValueOnce(new Error('cleanup'))
    await expect(f.start()).rejects.toBe(error)
  })
})
