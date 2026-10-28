import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { clientComponent, MemoryNode as Node, memoryRenderer as renderer } from '../helpers/component-host'
import type { AnnotationEditor } from '../../src/features/annotations/use-annotations'

const api = vi.hoisted(() => ({ recordingAnnotations: vi.fn(), currentRecording: vi.fn(), updateRecordingAnnotations: vi.fn(), setRecordingAnnotationMode: vi.fn(), controlRecording: vi.fn() }))
const events = vi.hoisted(() => ({ closed: undefined as undefined | ((event: { payload: number }) => void), off: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_name: string, cb: typeof events.closed) => { events.closed = cb; return events.off } }))
vi.mock('../../src/features/annotations/toolbar-layout', () => ({ useFloatingToolbar: () => ({ style: {}, start: vi.fn(), reset: vi.fn() }), annotationToolbarPosition: () => ({}) }))
vi.mock('../../src/features/annotations/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h, onMounted } = await import('vue')
  return { default: defineComponent({ props: ['editor', 'toolbarVisible', 'defaultExpanded', 'active', 'surfaceVisible', 'pointerLabel'], emits: ['toolbarElement', 'tool'], setup: (props, { slots, emit }) => {
    onMounted(() => emit('toolbarElement', { getBoundingClientRect: () => ({ x: 100, y: 600, left: 100, top: 600, width: 800, height: 100 }) }))
    return () => h('workspace', { editor: props.editor, toolbarVisible: props.toolbarVisible, defaultExpanded: props.defaultExpanded, active: props.active, surfaceVisible: props.surfaceVisible, pointerLabel: props.pointerLabel, onChoose: () => emit('tool', 'arrow'), onDesktop: () => emit('tool', 'edit') }, props.toolbarVisible ? [slots.actions?.(), slots.footer?.()] : [])
  } }) }
})
import Overlay from '../../src/features/recording/components/RecordingAnnotations.vue'
const OverlayClient = clientComponent(Overlay, new URL('../../src/features/recording/components/RecordingAnnotations.vue', import.meta.url))
const node = (type: string) => new Node(type)
const root = node('root')
function find(test: (n: Node) => boolean, current = root): Node | undefined {
  if (test(current)) return current
  for (const child of current.children) { const found = find(test, child); if (found) return found }
}
const initial = { id: 7, revision: 0, width: 640, height: 480, viewportWidth: 1920, viewportHeight: 1080, x: 100, y: 100, editing: true, shapes: [] }
const workspace = () => find(n => n.type === 'workspace')
const editor = () => workspace()!.props.editor as AnnotationEditor
function press(key: string) {
  const handler = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'keydown')![1] as (event: KeyboardEvent) => void
  const event = { key, preventDefault: vi.fn(), target: { closest: () => null } } as unknown as KeyboardEvent
  handler(event)
  return event
}
async function flush() { await vi.advanceTimersByTimeAsync(0); await Vue.nextTick() }
async function mount(exposed?: Vue.Ref<InstanceType<typeof Overlay> | null>) {
  const Wrapper = { setup() { Vue.provide(Vue.ssrContextKey, { modules: new Set() }); return () => exposed ? Vue.h(Overlay, { ref: exposed }) : Vue.h(OverlayClient) } }
  renderer.render(Vue.h(Wrapper), root); await flush()
}
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('innerWidth', 1920); vi.stubGlobal('innerHeight', 1080)
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  api.recordingAnnotations.mockResolvedValue({ ...initial })
  api.currentRecording.mockResolvedValue({ id: 7, phase: 'recording', seconds: 3 })
  api.updateRecordingAnnotations.mockResolvedValue({ ...initial, revision: 1, shapes: undefined })
  api.setRecordingAnnotationMode.mockResolvedValue(undefined)
})
afterEach(() => { renderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('共享标注页面的录制宿主', () => {
  it('确认范围后就使用桌面模式接管，不在待开始阶段展开全屏标注层', async () => {
    api.recordingAnnotations.mockResolvedValue({ ...initial, editing: false })
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    const exposed = Vue.ref<InstanceType<typeof Overlay> | null>(null)
    await mount(exposed)
    await exposed.value!.prepareDesktop(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false, { x: 100, y: 600, width: 800, height: 100 })
    expect(api.setRecordingAnnotationMode.mock.calls.every(([, editing]) => editing === false)).toBe(true)
    expect(workspace()?.props.surfaceVisible).toBe(false)
    expect(workspace()?.props.toolbarVisible).toBe(true)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('首屏查询失败不能退回全屏交互模式显示窗口', async () => {
    api.recordingAnnotations.mockResolvedValue(null)
    const exposed = Vue.ref<InstanceType<typeof Overlay> | null>(null)
    await mount(exposed)
    await expect(exposed.value!.prepareDesktop()).rejects.toThrow('录屏工具栏尚未就绪')
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
  })
  it('录制中的指针工具交还页面点击和滚动，再次点击也不会开启标注', async () => {
    await mount()
    expect(workspace()?.props.pointerLabel).toBe('操作桌面')
    editor().setTool('pen')
    editor().begin({ x: 10, y: 10 }, 1); editor().drag({ x: 40, y: 40 }); await editor().finish()
    ;(workspace()!.props.onDesktop as () => void)(); await flush()
    expect(workspace()?.props.surfaceVisible).toBe(false)
    expect(editor().history.present).toHaveLength(1)
    ;(workspace()!.props.onDesktop as () => void)(); await flush()
    expect(workspace()?.props.active).toBe(false)
    expect(api.setRecordingAnnotationMode.mock.calls.every(([, editing]) => editing === false)).toBe(true)
    ;(workspace()!.props.onChoose as () => void)(); await flush()
    expect(workspace()?.props.surfaceVisible).toBe(true)
    expect(editor().tool).toBe('arrow')
  })
  it('截图窗口尚未显示时也挂载开始按钮，不等待显示事件或十秒轮询', async () => {
    vi.stubGlobal('document', { visibilityState: 'hidden', addEventListener: vi.fn(), removeEventListener: vi.fn() })
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    await mount()
    expect(workspace()).toBeDefined()
    expect(find(n => n.type === 'button' && n.props.title === '开始录制')).toBeDefined()
    expect(api.controlRecording).not.toHaveBeenCalled()
    press(' '); await flush()
    expect(api.controlRecording).toHaveBeenLastCalledWith(7, 'start')
  })
  it('Ready 不自动开始，空格依次开始、暂停、继续', async () => {
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    await mount()
    expect(api.controlRecording).not.toHaveBeenCalled()
    expect(find(n => n.type === 'button' && n.children.some(c => c.text.includes('开始录制')))).toBeDefined()
    for (const [phase, action] of [['recording', 'start'], ['paused', 'pause'], ['recording', 'resume']]) {
      const modeCalls = api.setRecordingAnnotationMode.mock.calls.length
      api.currentRecording.mockResolvedValue({ id: 7, phase, seconds: 0 })
      const event = press(' '); await flush()
      expect(event.preventDefault).toHaveBeenCalled()
      expect(api.controlRecording).toHaveBeenLastCalledWith(7, action)
      if (action !== 'pause') {
        // 开始/继续必须真正提交，不能被上次裁剪的缓存跳过。
        expect(api.setRecordingAnnotationMode.mock.calls.length).toBeGreaterThan(modeCalls)
        expect(api.setRecordingAnnotationMode).toHaveBeenCalledWith(7, false, { x: 100, y: 600, width: 800, height: 100 })
        expect(workspace()?.props.active).toBe(false)
      }
    }
  })
  it('全局 Esc 停止录制并移除整个工具栏，迟到轮询不会重新显示', async () => {
    await mount()
    press('Escape'); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(7, 'stop')
    expect(workspace()).toBeUndefined()
    await vi.advanceTimersByTimeAsync(250)
    expect(workspace()).toBeUndefined()
  })
  it('输入框内空格仍用于输入，长按和组词不反复控制录制', async () => {
    await mount()
    const handler = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'keydown')![1] as (event: KeyboardEvent) => void
    for (const patch of [{ target: { closest: () => ({}) } }, { repeat: true }, { isComposing: true }, { defaultPrevented: true }]) {
      const event = { key: ' ', preventDefault: vi.fn(), target: { closest: () => null }, ...patch } as unknown as KeyboardEvent
      handler(event)
    }
    await flush()
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('待开始时 Esc 取消会话；停止失败则保留工具栏供重试', async () => {
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    api.controlRecording.mockRejectedValueOnce(new Error('退出失败'))
    await mount()
    press('Escape'); await flush()
    expect(workspace()).toBeDefined()
    expect(api.controlRecording).toHaveBeenCalledWith(7, 'cancel')
    press('Escape'); await flush()
    expect(workspace()).toBeUndefined()
  })
  it('提交后保留撤销栈，迟到轮询不能恢复旧图形', async () => {
    await mount()
    expect(workspace()?.props.defaultExpanded).toBe('')
    const e = editor()
    e.setTool('pen')
    e.begin({ x: 10, y: 10 }, 1); e.drag({ x: 40, y: 40 }); await e.finish()
    expect(api.updateRecordingAnnotations).toHaveBeenCalledWith(7, expect.arrayContaining([expect.objectContaining({ kind: 'pen' })]))
    await vi.advanceTimersByTimeAsync(250)
    expect(editor().history.present).toHaveLength(1)
    expect(editor().history.past).toHaveLength(1)
  })
  it('桌面模式选工具恢复绘制，重复点击当前工具交还桌面点击', async () => {
    api.recordingAnnotations.mockResolvedValueOnce({ ...initial, editing: false })
    await mount()
    expect(find(n => n.type === 'button' && n.children.some(c => c.text.includes('继续操作桌面')))).toBeUndefined()
    expect(workspace()?.props.toolbarVisible).toBe(true)
    expect(api.controlRecording).not.toHaveBeenCalled()
    ;(workspace()!.props.onChoose as () => void)(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, true, undefined)
    expect(editor().tool).toBe('arrow')
    expect(workspace()?.props.active).toBe(true)
    ;(workspace()!.props.onChoose as () => void)(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false, { x: 100, y: 600, width: 800, height: 100 })
    expect(workspace()?.props.active).toBe(false)
  })
  it('开始前裁剪失败不启动录制，工具栏保留重试入口', async () => {
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    await mount()
    api.setRecordingAnnotationMode.mockRejectedValueOnce(new Error('窗口裁剪失败'))
    press(' '); await flush()
    expect(api.controlRecording).not.toHaveBeenCalled()
    expect(workspace()?.props.active).toBe(true)
    expect(find(n => n.props.role === 'alert')?.text).toContain('窗口裁剪失败')
    press(' '); await flush()
    expect(api.controlRecording).toHaveBeenLastCalledWith(7, 'start')
  })
  it('切换标注期间发出的旧桌面快照不能在切换成功后收回画布', async () => {
    api.recordingAnnotations.mockResolvedValueOnce({ ...initial, editing: false })
    await mount()
    let completeMode!: () => void
    api.setRecordingAnnotationMode.mockReturnValueOnce(new Promise<void>(resolve => { completeMode = resolve }))
    ;(workspace()!.props.onChoose as () => void)(); await flush()
    let completePoll!: (view: typeof initial) => void
    api.recordingAnnotations.mockReturnValueOnce(new Promise(resolve => { completePoll = resolve }))
    await vi.advanceTimersByTimeAsync(1000)
    completeMode(); await flush()
    expect(workspace()?.props.active).toBe(true)
    completePoll({ ...initial, editing: false }); await flush()
    expect(workspace()?.props.active).toBe(true)
    expect(editor().tool).toBe('arrow')
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, true, undefined)
    editor().begin({ x: 10, y: 10 }, 1); editor().drag({ x: 40, y: 40 }); await editor().finish()
    expect(api.updateRecordingAnnotations).toHaveBeenLastCalledWith(7, expect.arrayContaining([expect.objectContaining({ kind: 'arrow' })]))
  })
  it('桌面裁剪按当前视口换算 DPI，调整视口后重新同步', async () => {
    api.recordingAnnotations.mockResolvedValue({ ...initial, editing: false })
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    await mount()
    // initialize 的临时模式不能与 capture 宿主接管窗口的请求竞争。
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
    press(' '); await flush()
    vi.stubGlobal('innerWidth', 1280); vi.stubGlobal('innerHeight', 720)
    const resize = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'resize')![1] as () => void
    resize(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false, { x: 150, y: 900, width: 1200, height: 150 })
    const modeCalls = api.setRecordingAnnotationMode.mock.calls.length
    resize(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenCalledTimes(modeCalls)
  })
  it('桌面操作模式下撤销仍立即保存，不需要先恢复绘制', async () => {
    await mount()
    editor().setTool('pen')
    editor().begin({ x: 10, y: 10 }, 1); editor().drag({ x: 40, y: 40 }); await editor().finish()
    api.recordingAnnotations.mockResolvedValueOnce({ ...initial, revision: 1, editing: false, shapes: undefined })
    await vi.advanceTimersByTimeAsync(250)
    api.updateRecordingAnnotations.mockResolvedValueOnce({ ...initial, revision: 2, editing: false, shapes: undefined })
    await editor().travel('undo')
    expect(api.updateRecordingAnnotations).toHaveBeenLastCalledWith(7, [])
    expect(editor().history.present).toEqual([])
    expect(workspace()?.props.toolbarVisible).toBe(true)
  })
  it('关闭事件阻止迟到查询重新挂载旧会话', async () => {
    let resolve!: (v: typeof initial) => void
    api.recordingAnnotations.mockReturnValueOnce(new Promise(r => { resolve = r }))
    await mount()
    events.closed!({ payload: 7 })
    resolve(initial); await flush()
    expect(workspace()).toBeUndefined()
  })
})
