import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { useAutoSave } from '../../src/shared/ui/use-auto-save'

afterEach(() => { vi.useRealTimers() })
describe('设置自动保存', () => {
  it('连续输入合并提交，失焦或开始操作前可立即提交', async () => {
    vi.useFakeTimers()
    const scope = effectScope(), save = vi.fn(async () => {})
    const state = scope.run(() => useAutoSave(save))!
    state.schedule(); await vi.advanceTimersByTimeAsync(200); state.schedule()
    await vi.advanceTimersByTimeAsync(300); expect(save).not.toHaveBeenCalled()
    await state.flush(); expect(save).toHaveBeenCalledOnce()
    expect(state.saved.value).toBe(true)
    scope.stop()
  })
  it('提交期间的新改动串行保存，不丢掉最后一次输入', async () => {
    vi.useFakeTimers()
    const scope = effectScope()
    let finish!: () => void
    const save = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve })).mockResolvedValue(undefined)
    const state = scope.run(() => useAutoSave(save))!
    state.schedule(); const running = state.flush(); await Promise.resolve()
    state.schedule(); finish(); await running
    expect(save).toHaveBeenCalledTimes(2)
    scope.stop()
  })
  it('无效输入不提交，失败可重试，离开页面仍提交待保存输入', async () => {
    vi.useFakeTimers()
    const scope = effectScope(), save = vi.fn().mockRejectedValueOnce(new Error('写入失败')).mockResolvedValue(undefined)
    let invalid: string | null = '前缀不能为空'
    const state = scope.run(() => useAutoSave(save, () => invalid))!
    state.schedule(); await state.flush(); expect(save).not.toHaveBeenCalled()
    expect(state.error.value).toBe(invalid)
    invalid = null; await state.retry(); expect(state.error.value).toContain('写入失败')
    await state.retry(); expect(state.saved.value).toBe(true)
    state.schedule(); scope.stop(); await Promise.resolve(); await Promise.resolve()
    expect(save).toHaveBeenCalledTimes(3)
  })
})
