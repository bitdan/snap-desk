import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick } from 'vue'
import { defaultRecordingPreferences, normalizeRecordingPreferences, readRecordingPreferences, saveRecordingPreferences } from '../../src/features/recording/preferences'
import { useRecordingSetup } from '../../src/features/recording/use-recording-setup'

const storage = () => {
  let content: string | null = null
  return { getItem: () => content, setItem: (_key: string, value: string) => { content = value } }
}
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })
describe('录屏偏好持久化', () => {
  it('重开后恢复有效偏好，并隔离颜色数组及音频开关', () => {
    const store = storage(), prefs = defaultRecordingPreferences()
    prefs.effects.keyboard = true; prefs.format = 'gif'; prefs.gifFps = 20; prefs.hideMainWindow = false
    saveRecordingPreferences({ ...prefs, systemAudio: true, microphone: true } as typeof prefs, store)
    const loaded = readRecordingPreferences(store)
    expect(loaded).toEqual(prefs)
    loaded.effects.leftColor[0] = 0
    expect(readRecordingPreferences(store).effects.leftColor[0]).toBe(64)
    expect(store.getItem()).not.toContain('systemAudio')
    expect(store.getItem()).not.toContain('microphone')
  })
  it('损坏、未来版本和不可写缓存不会阻止录屏', () => {
    expect(readRecordingPreferences({ ...storage(), getItem: () => '{' })).toEqual(defaultRecordingPreferences())
    expect(readRecordingPreferences({ ...storage(), getItem: () => '{"version":999}' })).toEqual(defaultRecordingPreferences())
    expect(saveRecordingPreferences(defaultRecordingPreferences(), { ...storage(), setItem: () => { throw new Error('quota') } })).toBe(false)
    const result = normalizeRecordingPreferences({ mp4Fps: 20, mbps: Infinity, effects: { size: 999, leftColor: [-1, 0, 0], clicks: 'true' } })
    expect(result).toEqual(defaultRecordingPreferences())
  })
  it('格式切换分别保留帧率，滑块改动合并写入，销毁时保存最后一笔', async () => {
    vi.useFakeTimers()
    const store = storage(), write = vi.spyOn(store, 'setItem')
    vi.stubGlobal('localStorage', store)
    const scope = effectScope(), setup = scope.run(useRecordingSetup)!
    setup.recordingFps.value = 30
    setup.recordingFormat.value = 'gif'; expect(setup.recordingFps.value).toBe(10)
    setup.recordingFps.value = 20
    setup.recordingFormat.value = 'mp4'; expect(setup.recordingFps.value).toBe(30)
    setup.recordingEffects.value.size = 30; await nextTick()
    setup.recordingEffects.value.size = 35; await nextTick()
    expect(write).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(200); expect(write).toHaveBeenCalledOnce()
    setup.recordingEffects.value.size = 40; await nextTick(); scope.stop()
    expect(readRecordingPreferences(store).effects.size).toBe(40)
    expect(readRecordingPreferences(store).gifFps).toBe(20)
    const second = effectScope(), restored = second.run(useRecordingSetup)!
    expect(restored.recordingSystemAudio.value).toBe(false)
    expect(restored.recordingMicrophone.value).toBe(false)
    second.stop()
  })
  it('收起主界面开关跨会话保留，旧偏好及无效值继续沿用默认行为', async () => {
    vi.useFakeTimers()
    const store = storage(); vi.stubGlobal('localStorage', store)
    expect(normalizeRecordingPreferences({}).hideMainWindow).toBe(true)
    expect(normalizeRecordingPreferences({ hideMainWindow: 'false' }).hideMainWindow).toBe(true)
    const first = effectScope(), setup = first.run(useRecordingSetup)!
    setup.hideMainWindow.value = false; await nextTick(); await vi.advanceTimersByTimeAsync(200)
    expect(readRecordingPreferences(store).hideMainWindow).toBe(false)
    first.stop()
    const second = effectScope(), restored = second.run(useRecordingSetup)!
    expect(restored.hideMainWindow.value).toBe(false)
    second.stop()
  })
})
