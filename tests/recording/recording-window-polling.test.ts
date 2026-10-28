import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import { useRecordingPolling } from '../../src/features/recording/use-recording-polling'

const native = vi.hoisted(() => ({ isVisible: vi.fn(), isMinimized: vi.fn(), onFocusChanged: vi.fn(), onResized: vi.fn() }))
const events = vi.hoisted(() => ({ subscribe: vi.fn(), changed: undefined as undefined | (() => void), off: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => native }))
vi.mock('../../src/features/recording/events', () => ({ onRecordingChanged: events.subscribe }))
const root = new MemoryNode('root'), refresh = vi.fn()
const Component = defineComponent({ setup() { useRecordingPolling(refresh, () => 'recording', vi.fn()); return () => h('div') } })
const InitialSnapshot = defineComponent({ setup() { useRecordingPolling(refresh, () => 'ready', vi.fn(), { initialSnapshot: true }); return () => h('div') } })

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  refresh.mockResolvedValue(undefined); native.isVisible.mockResolvedValue(true); native.isMinimized.mockResolvedValue(false)
  native.onFocusChanged.mockResolvedValue(vi.fn()); native.onResized.mockResolvedValue(vi.fn())
  events.subscribe.mockImplementation(async callback => { events.changed = callback; return events.off })
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })
describe('原生窗口可见性与录屏失效事件', () => {
  it('隐藏时挂载的工具栏先读取首屏快照，后续隐藏轮询仍停止业务查询', async () => {
    native.isVisible.mockResolvedValue(false)
    vi.stubGlobal('document', { visibilityState: 'hidden', addEventListener: vi.fn(), removeEventListener: vi.fn() })
    memoryRenderer.render(h(InitialSnapshot), root); await flushComponent()
    expect(refresh).toHaveBeenCalledOnce()
    events.changed!(); await flushComponent()
    await vi.advanceTimersByTimeAsync(20000)
    expect(refresh).toHaveBeenCalledOnce()
    vi.stubGlobal('document', { visibilityState: 'visible', addEventListener: vi.fn(), removeEventListener: vi.fn() })
    native.isVisible.mockResolvedValue(true)
    events.changed!(); await flushComponent()
    expect(refresh).toHaveBeenCalledTimes(2)
  })
  it('隐藏和最小化窗口不查询业务状态，重新显示后的通知立即同步', async () => {
    native.isVisible.mockResolvedValue(false)
    memoryRenderer.render(h(Component), root); await flushComponent()
    expect(refresh).not.toHaveBeenCalled()
    native.isVisible.mockResolvedValue(true); native.isMinimized.mockResolvedValue(true)
    events.changed!(); await flushComponent(); expect(refresh).not.toHaveBeenCalled()
    native.isMinimized.mockResolvedValue(false)
    events.changed!(); await flushComponent(); expect(refresh).toHaveBeenCalledOnce()
  })
  it('事件立即合并刷新，并在卸载时注销所有订阅', async () => {
    memoryRenderer.render(h(Component), root); await flushComponent()
    events.changed!(); await flushComponent(); expect(refresh).toHaveBeenCalledTimes(2)
    memoryRenderer.render(null, root); events.changed!(); await flushComponent()
    expect(refresh).toHaveBeenCalledTimes(2)
    expect(events.off).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('窗口已卸载时迟到的监听器注册会立即释放', async () => {
    let complete!: (off: () => void) => void
    events.subscribe.mockReturnValueOnce(new Promise(resolve => { complete = resolve }))
    memoryRenderer.render(h(Component), root); await flushComponent()
    memoryRenderer.render(null, root); complete(events.off); await flushComponent()
    expect(events.off).toHaveBeenCalledOnce()
  })
})
