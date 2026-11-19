import { describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
const cancel = vi.hoisted(() => vi.fn())
vi.mock('@/services/api', () => ({ api: { cancel } }))
import { useCaptureActions } from '../../src/features/capture/use-capture-actions'
import type { CaptureFrame } from '../../src/features/capture/types'

function setup(frame: CaptureFrame | null, scroll: unknown = null) {
  const state = {
    frame: ref(frame), pixels: ref(null), valid: ref(null), busy: ref(false), error: ref(''), scroll: ref(scroll),
  }
  const releaseFrame = vi.fn()
  const actions = useCaptureActions({
    ...state, blocked: () => false, marked: () => false, editorPending: () => false,
    recognitionOpen: () => false, annotations: () => [], releaseFrame, remember: vi.fn(),
  })
  return { ...state, ...actions, releaseFrame }
}

describe('截图取消', () => {
  it('前端已释放帧时仍请求后端收起遗留蒙层，失败不打扰用户', async () => {
    cancel.mockReset().mockRejectedValueOnce('截图已过期。')
    const { cancel: run, error, busy } = setup(null)
    await run()
    expect(cancel).toHaveBeenCalledWith(0)
    expect(error.value).toBe('')
    expect(busy.value).toBe(false)
  })

  it('滚动截图接管期间不发送遗留蒙层清理', async () => {
    cancel.mockReset()
    await setup(null, { id: 3 }).cancel()
    expect(cancel).not.toHaveBeenCalled()
  })

  it('正常会话按编号取消并释放帧', async () => {
    cancel.mockReset().mockResolvedValueOnce(undefined)
    const { cancel: run, releaseFrame } = setup({ id: 7 } as CaptureFrame)
    await run()
    expect(cancel).toHaveBeenCalledWith(7)
    expect(releaseFrame).toHaveBeenCalledWith(7)
  })
})
