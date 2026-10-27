import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createRecordingPoller, recordingPollInterval } from '../../src/features/recording/polling'

beforeEach(() => vi.useFakeTimers())
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers() })
describe('录屏窗口刷新调度', () => {
  it('隐藏期间只探测可见性，唤醒后立即获取状态', async () => {
    const refresh = vi.fn().mockResolvedValue(undefined), visible = vi.fn().mockResolvedValue(false)
    const poller = createRecordingPoller({ refresh, visible, interval: () => 1000, onError: vi.fn() })
    await poller.wake(); await vi.advanceTimersByTimeAsync(20000)
    expect(refresh).not.toHaveBeenCalled()
    expect(visible).toHaveBeenCalledTimes(3)
    visible.mockResolvedValue(true); await poller.wake()
    expect(refresh).toHaveBeenCalledOnce()
    poller.stop()
  })
  it('大量通知不会并发查询，只在当前请求完成后补刷一次', async () => {
    let complete!: () => void
    const refresh = vi.fn().mockReturnValueOnce(new Promise<void>(resolve => { complete = resolve })).mockResolvedValue(undefined)
    const poller = createRecordingPoller({ refresh, visible: async () => true, interval: () => 1000, onError: vi.fn() })
    const first = poller.wake(); await Promise.resolve()
    for (let i = 0; i < 20; i++) void poller.wake()
    expect(refresh).toHaveBeenCalledOnce()
    complete(); await first; await vi.advanceTimersByTimeAsync(0)
    expect(refresh).toHaveBeenCalledTimes(2)
    poller.stop()
  })
  it('请求未完成时卸载，迟到结果不能重新创建计时器', async () => {
    let complete!: () => void
    const refresh = vi.fn(() => new Promise<void>(resolve => { complete = resolve }))
    const poller = createRecordingPoller({ refresh, visible: async () => true, interval: () => 1000, onError: vi.fn() })
    const first = poller.wake(); await Promise.resolve(); poller.stop(); complete(); await first
    await vi.advanceTimersByTimeAsync(30000)
    expect(refresh).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('失败后继续校准，阶段结束后降低查询频率', async () => {
    const onError = vi.fn(), refresh = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(undefined)
    const poller = createRecordingPoller({ refresh, visible: async () => true, interval: () => recordingPollInterval('completed'), onError })
    await poller.wake(); expect(onError).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(9999); expect(refresh).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(1); expect(refresh).toHaveBeenCalledTimes(2)
    expect(recordingPollInterval('countdown')).toBeLessThan(recordingPollInterval('recording'))
    poller.stop()
  })
})
