import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ currentRecording: vi.fn(), latestRecordingResult: vi.fn(),
  currentRecordingDirectory: vi.fn(), recordingCapabilities: vi.fn(), recoverableRecordings: vi.fn(),
  recordingResultAction: vi.fn(), selectRecordingArea: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Panel from '../../src/features/recording/components/RecordingPanel.vue'
const Component = clientComponent(Panel, new URL('../../src/features/recording/components/RecordingPanel.vue', import.meta.url))
const root = new MemoryNode('root')
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
const button = (label: string) => findNode(root, node => node.type === 'button' && node.children.some(child => child.text === label))

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  api.currentRecording.mockResolvedValue(null)
  api.latestRecordingResult.mockResolvedValue({ id: 7, filename: 'recording.mp4', path: 'D:/recording.mp4' })
  api.currentRecordingDirectory.mockResolvedValue({ path: 'D:/', isDefault: true })
  api.recordingCapabilities.mockResolvedValue({ available: true })
  api.recoverableRecordings.mockResolvedValue([])
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers() })

describe('录屏页只展示成品与录制入口', () => {
  it('最近完成的录屏提供打开目录和复制路径，不挂载播放器或项目编辑器', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    expect(button('打开目录')).toBeDefined()
    expect(button('复制路径')).toBeDefined()
    expect(button('刷新')).toBeUndefined()
    expect(findNode(root, node => node.text.includes('可恢复录制') || node.text.includes('暂无待恢复录制'))).toBeUndefined()
    for (const label of ['打开录屏项目', '预览与裁剪', '预览（系统播放器）']) expect(button(label)).toBeUndefined()
    expect(findNode(root, node => node.type === 'video')).toBeUndefined()
    button('打开目录')!.props.onClick(); await flush()
    expect(api.recordingResultAction).toHaveBeenCalledWith(7, 'openDirectory')
    button('复制路径')!.props.onClick(); await flush()
    expect(api.recordingResultAction).toHaveBeenCalledWith(7, 'copyPath')
  })
  it('保留选择录制范围入口，直接进入既有录制流程', async () => {
    memoryRenderer.render(h(Component), root); await flush()
    button('选择录制范围')!.props.onClick(); await flush()
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
  })
})
