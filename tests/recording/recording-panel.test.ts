import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defaultSettings } from '../../src/features/settings/settings'
import { h, nextTick } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), currentRecording: vi.fn(), latestRecordingResult: vi.fn(),
  currentRecordingDirectory: vi.fn(), recordingCapabilities: vi.fn(), recoverableRecordings: vi.fn(),
  recordingResultAction: vi.fn(), selectRecordingArea: vi.fn(), controlRecording: vi.fn() }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Preferences from '../../src/features/recording/components/RecordingPreferences.vue'
import FileSettings from '../../src/features/recording/components/RecordingFileSettings.vue'
clientComponent(FileSettings, new URL('../../src/features/recording/components/RecordingFileSettings.vue', import.meta.url))
import Effects from '../../src/features/recording/components/RecordingInputSettings.vue'
import { defaultRecordingPreferences, readRecordingPreferences, saveRecordingPreferences, RECORDING_PREFERENCES_KEY } from '../../src/features/recording/preferences'
clientComponent(Preferences, new URL('../../src/features/recording/components/RecordingPreferences.vue', import.meta.url))
clientComponent(Effects, new URL('../../src/features/recording/components/RecordingInputSettings.vue', import.meta.url))
import Panel from '../../src/features/recording/components/RecordingPanel.vue'
const Component = clientComponent(Panel, new URL('../../src/features/recording/components/RecordingPanel.vue', import.meta.url))
const root = new MemoryNode('root')
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
const button = (label: string) => findNode(root, node => node.type === 'button' && node.children.some(child => child.text === label))

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('Document', class {})
  vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  let content: string | null = null
  vi.stubGlobal('localStorage', { getItem: () => content, setItem: (_key: string, value: string) => { content = value } })
  const settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings: structuredClone(settings) }))
  api.saveSettings.mockImplementation(async value => { Object.assign(settings, value); return structuredClone(settings) })
  api.currentRecording.mockResolvedValue(null)
  api.latestRecordingResult.mockResolvedValue({ id: 7, filename: 'recording.mp4', path: 'D:/recording.mp4' })
  api.currentRecordingDirectory.mockResolvedValue({ path: 'D:/', isDefault: true })
  api.recordingCapabilities.mockResolvedValue({ available: true })
  api.recoverableRecordings.mockResolvedValue([{ id: 'pending', path: 'D:/pending/video.mp4', finalized: true, sourceExists: true, exported: null }])
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals() })

describe('录屏页设置与录制入口', () => {
  it('仅显示录制设置与控制，不查询或展示最近录屏和恢复记录', async () => {
    api.currentRecording.mockResolvedValue({ id: 9, revision: 1, phase: 'recording', seconds: 12, systemAudio: false, microphone: false, message: '' })
    memoryRenderer.render(h(Component), root); await flush()
    for (const label of ['打开目录', '复制路径', '重试导出', '清理残留']) expect(button(label)).toBeUndefined()
    expect(findNode(root, node => node.text.includes('最近完成的录屏') || node.text.includes('可恢复录制'))).toBeUndefined()
    expect(api.latestRecordingResult).not.toHaveBeenCalled()
    expect(api.recoverableRecordings).not.toHaveBeenCalled()

    expect(button('停止并导出')).toBeDefined()
    api.currentRecording.mockResolvedValue({ id: 9, revision: 2, phase: 'completed', seconds: 12, message: '' })
    button('停止并导出')!.props.onClick(); await flush()
    expect(button('选择录制范围')).toBeDefined()
    expect(api.latestRecordingResult).not.toHaveBeenCalled()
    expect(api.recoverableRecordings).not.toHaveBeenCalled()
  })
  it('保留选择录制范围入口，直接进入既有录制流程', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
  })
  it('主页面设置在进入选区前保存，无效码率阻止进入', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    const bitrate = findNode(root, node => node.props.id === 'record-bitrate')!
    bitrate.props['onUpdate:modelValue']('100'); await nextTick()
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).not.toHaveBeenCalled()
    bitrate.props['onUpdate:modelValue']('12.5'); await nextTick()
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
    expect(readRecordingPreferences()).toMatchObject({ mbps: 12.5 })
  })
  it('缓存不可写时不进入使用旧配置的选区', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => { throw new Error('quota') } })
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).not.toHaveBeenCalled()
    expect(findNode(root, node => node.props.role === 'status' && node.text.includes('无法保存录屏偏好'))).toBeDefined()
  })

  it('录制中锁住设置，暂停与停止直接控制当前会话', async () => {
    api.currentRecording.mockResolvedValue({ id: 9, revision: 1, phase: 'recording', seconds: 12, systemAudio: true, microphone: false, message: '' })
    memoryRenderer.render(h(Component), root); await flush()
    expect(button('选择录制范围')).toBeUndefined()
    expect(findNode(root, node => node.type === 'fieldset')?.props.disabled).toBe(true)
    button('暂停')!.props.onClick(); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(9, 'pause')
    button('停止并导出')!.props.onClick(); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(9, 'stop')
  })
  it('录屏能力读取失败后可重试，不持续显示检查中', async () => {
    api.recordingCapabilities.mockRejectedValueOnce(new Error('设备检查失败'))
    memoryRenderer.render(h(Component), root); await flush()
    expect(button('选择录制范围')?.props.disabled).toBe(true)
    expect(button('重新读取')).toBeDefined()
    button('重新读取')!.props.onClick(); await flush()
    expect(api.recordingCapabilities).toHaveBeenCalledTimes(2)
    expect(button('选择录制范围')?.props.disabled).toBe(false)
  })

  it('选区窗口修改偏好后同步主页面，下一次录制保留新值', async () => {
    const events = new EventTarget()
    vi.stubGlobal('window', events)
    memoryRenderer.render(h(Component), root); await flush()
    saveRecordingPreferences({ ...defaultRecordingPreferences(), format: 'gif', gifFps: 20 })
    events.dispatchEvent(Object.assign(new Event('storage'), { key: RECORDING_PREFERENCES_KEY }))
    await nextTick()
    const format = findNode(root, node => node.props.id === 'record-format')!
    const fps = findNode(root, node => node.props.id === 'record-fps')!
    expect(format.value).toBe('gif')
    expect(fps.value).toBe(20)
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
    expect(readRecordingPreferences()).toMatchObject({ format: 'gif', gifFps: 20 })
  })

  it('录屏文件命名自动保存，进入选区前提交最后一次修改', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    const prefix = findNode(root, node => node.props.id === 'record-prefix')!
    prefix.props['onUpdate:modelValue']('Demo')
    findNode(root, node => node.type === 'form')!.props.onInput()
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ recordingExport: { filenamePrefix: 'Demo', includeDimensions: false, rememberDirectory: true } }))
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
    expect(button('保存录屏偏好')).toBeUndefined()
  })

})
