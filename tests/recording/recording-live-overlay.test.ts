import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { clientComponent, MemoryNode as Node, memoryRenderer as renderer } from '../helpers/component-host'
import { defaultRecordingAnnotationTools } from '../../src/features/recording/recording-annotations'
import type { AnnotationEditor } from '../../src/features/annotations/use-annotations'

const api = vi.hoisted(() => ({ recordingAnnotations: vi.fn(), currentRecording: vi.fn(), updateRecordingAnnotations: vi.fn(), setRecordingAnnotationMode: vi.fn(), controlRecording: vi.fn(), publishRecordingAnnotationEditor: vi.fn(), finishRecordingAnnotationEditorAction: vi.fn() }))
const events = vi.hoisted(() => ({ closed: undefined as undefined | ((event: { payload: number }) => void), off: vi.fn(), editor: undefined as undefined | ((event: {payload: import('../../src/features/recording/recording-annotations').RecordingAnnotationEditorRequest}) => void) }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async (name: string, cb: typeof events.closed) => { if(name==='recording-annotations-closed') events.closed=cb; else events.editor=cb as unknown as typeof events.editor; return events.off } }))
vi.mock('../../src/features/annotations/toolbar-layout', () => ({ useFloatingToolbar: () => ({ style: {}, start: vi.fn(), reset: vi.fn() }), annotationToolbarPosition: () => ({}) }))
vi.mock('../../src/features/annotations/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h, onMounted } = await import('vue')
  return { default: defineComponent({ props: ['editor', 'toolbarVisible', 'defaultExpanded', 'active', 'surfaceVisible', 'pointerLabel', 'enabled', 'presentation'], emits: ['toolbarElement', 'tool'], setup: (props, { slots, emit }) => {
    onMounted(() => emit('toolbarElement', { getBoundingClientRect: () => ({ x: 100, y: 600, left: 100, top: 600, width: 800, height: 100 }) }))
    return () => h('workspace', { editor: props.editor, presentation: props.presentation, toolbarVisible: props.toolbarVisible, defaultExpanded: props.defaultExpanded, active: props.active, surfaceVisible: props.surfaceVisible, enabled: props.enabled, pointerLabel: props.pointerLabel, onChoose: () => emit('tool', 'arrow'), onDesktop: () => emit('tool', 'edit') }, props.toolbarVisible ? [slots.actions?.(), slots.footer?.()] : [])
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
const initial = { id: 7, revision: 0, width: 640, height: 480, viewportWidth: 1920, viewportHeight: 1080, x: 100, y: 100, editing: true, shapes: [], mediaTimeMs: 0, tools: defaultRecordingAnnotationTools(), tracks: [] }
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
  api.publishRecordingAnnotationEditor.mockResolvedValue(undefined)
  api.finishRecordingAnnotationEditorAction.mockResolvedValue(undefined)
})
afterEach(() => { renderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('独立的录屏标注层', () => {
  async function draw() {
    editor().setTool('pen')
    editor().begin({ x: 10, y: 10 }, 1); editor().drag({ x: 40, y: 40 }); await editor().finish()
  }
  it('确认范围交给独立控制条，没有标注时不显示画布和工具栏', async () => {
    api.recordingAnnotations.mockResolvedValue({ ...initial, editing: false })
    const exposed = Vue.ref<InstanceType<typeof Overlay> | null>(null)
    await mount(exposed)
    await exposed.value!.prepareDesktop(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false)
    expect(workspace()?.props.surfaceVisible).toBe(false)
    expect(workspace()?.props.toolbarVisible).toBe(false)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('首屏查询失败不能退回全屏交互模式', async () => {
    api.recordingAnnotations.mockResolvedValue(null)
    const exposed = Vue.ref<InstanceType<typeof Overlay> | null>(null)
    await mount(exposed)
    await expect(exposed.value!.prepareDesktop()).rejects.toThrow('录屏工具栏尚未就绪')
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
  })
  it('完成标注保留图形展示，关闭输入和工具栏，不暂停或结束录屏', async () => {
    await mount(); await draw()
    press('Escape'); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false)
    expect(workspace()?.props.surfaceVisible).toBe(true)
    expect(workspace()?.props.enabled).toBe(false)
    expect(workspace()?.props.toolbarVisible).toBe(false)
    expect(editor().history.present).toHaveLength(1)
    expect(editor().selected).toBe(-1)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('标注工具栏只负责标注，不再放置另一套录屏按钮', async () => {
    await mount()
    expect(workspace()?.props.toolbarVisible).toBe(false)
    expect(find(n => n.type === 'button')).toBeUndefined()
    for (const label of ['开始录制', '暂停录制', '结束录制并编辑']) {
      expect(find(n => n.type === 'button' && n.props.title === label)).toBeUndefined()
    }
    press(' '); await flush()
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('Esc 完成标注并交还桌面输入，不结束录屏', async () => {
    await mount(); await draw()
    press('Escape'); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, false)
    expect(workspace()?.props.surfaceVisible).toBe(true)
    expect(workspace()?.props.enabled).toBe(false)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('完成失败时保留绘制状态和重试入口', async () => {
    await mount(); await draw()
    api.setRecordingAnnotationMode.mockRejectedValueOnce(new Error('切换失败'))
    press('Escape'); await flush()
    expect(workspace()?.props.active).toBe(true)
    expect(find(n => n.props.role === 'alert')?.text).toContain('切换失败')
    press('Escape'); await flush()
    expect(workspace()?.props.active).toBe(false)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('独立工具窗切换桌面后同步输入状态，不清除已提交的图形', async () => {
    await mount(); await draw()
    api.recordingAnnotations.mockResolvedValue({ ...initial, revision: 1, editing: false, shapes: undefined })
    await vi.advanceTimersByTimeAsync(1000)
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
    expect(workspace()?.props.surfaceVisible).toBe(true)
    expect(workspace()?.props.toolbarVisible).toBe(false)
    expect(editor().history.present).toHaveLength(1)
  })
  it('独立控制条再次开启标注后保留历史，之后清空能同步展示层', async () => {
    await mount(); await draw()
    api.recordingAnnotations.mockResolvedValue({ ...initial, revision: 1, editing: false, shapes: undefined })
    await vi.advanceTimersByTimeAsync(1000)
    expect(workspace()?.props.surfaceVisible).toBe(true)
    expect(workspace()?.props.toolbarVisible).toBe(false)
    api.recordingAnnotations.mockResolvedValue({ ...initial, revision: 1, editing: true, shapes: undefined })
    await vi.advanceTimersByTimeAsync(1000)
    expect(workspace()?.props.active).toBe(true)
    expect(editor().history.present).toHaveLength(1)
    api.recordingAnnotations.mockResolvedValue({ ...initial, revision: 2, editing: false, shapes: [] })
    await vi.advanceTimersByTimeAsync(1000)
    expect(workspace()?.props.surfaceVisible).toBe(false)
    expect(editor().history.present).toHaveLength(0)
  })
  it('原生媒体时间驱动淡出，暂停的相同 PTS 不继续减少透明度', async () => {
    const shape={kind:'rectangle' as const,start:{x:10,y:10},end:{x:40,y:40},color:'#ff0000',width:2,opacity:1}
    const tracks=[{id:'a-1',shape,startMs:1000,endMs:4000,fadeOutMs:500,order:0}]
    api.recordingAnnotations.mockResolvedValue({...initial,editing:false,mediaTimeMs:3750,shapes:[shape],tracks})
    await mount()
    expect((workspace()?.props.presentation as typeof shape[])[0].opacity).toBe(.5)
    await vi.advanceTimersByTimeAsync(3000)
    expect((workspace()?.props.presentation as typeof shape[])[0].opacity).toBe(.5)
    api.recordingAnnotations.mockResolvedValue({...initial,editing:false,mediaTimeMs:4000,shapes:undefined,tracks:undefined})
    await vi.advanceTimersByTimeAsync(1000)
    expect(workspace()?.props.surfaceVisible).toBe(false)
    expect(editor().history.present).toHaveLength(1)
  })
  it('独立工具窗的工具与样式进入画布，不依赖本窗工具栏', async () => {
    api.recordingAnnotations.mockResolvedValue({...initial,tools:{...defaultRecordingAnnotationTools(),tool:'arrow',color:'#123456',width:8}})
    await mount()
    expect(editor().tool).toBe('arrow'); expect(editor().color).toBe('#123456'); expect(editor().width).toBe(8)
    expect(workspace()?.props.toolbarVisible).toBe(false)
  })
  it('截图工具的荧光笔默认样式直接用于录屏，不维护第二套默认值',async()=>{
    await mount()
    events.editor!({payload:{id:7,requestId:1,action:{kind:'tool',tool:'highlighter'}}});await flush()
    expect(editor().tool).toBe('highlighter')
    expect(editor().width).toBe(20);expect(editor().opacity).toBe(.35)
    expect(api.publishRecordingAnnotationEditor).toHaveBeenCalledWith(7,expect.any(Object),expect.objectContaining({tool:'highlighter',width:20,opacity:.35}))
  })
  it('共享截图工具栏的撤销重做使用画布同一份历史',async()=>{
    await mount();await draw();await flush()
    events.editor!({payload:{id:7,requestId:1,action:{kind:'undo'}}});await flush()
    expect(editor().history.present).toHaveLength(0)
    expect(editor().history.future).toHaveLength(1)
    expect(api.finishRecordingAnnotationEditorAction).toHaveBeenCalledWith(7,1,null)
    events.editor!({payload:{id:7,requestId:2,action:{kind:'redo'}}});await flush()
    expect(editor().history.present).toHaveLength(1)
    expect(api.finishRecordingAnnotationEditorAction).toHaveBeenCalledWith(7,2,null)
    expect(api.publishRecordingAnnotationEditor).toHaveBeenCalledWith(7,expect.objectContaining({ready:true,canUndo:true,canRedo:false}),expect.any(Object))
  })
  it('共享样式操作沿用原编辑器，修改选中图形后仍能撤销',async()=>{
    await mount();await draw();editor().select(0)
    const previous=editor().history.present[0].color
    events.editor!({payload:{id:7,requestId:1,action:{kind:'style',style:{color:'#123456'}}}});await flush()
    expect(editor().history.present[0].color).toBe('#123456')
    await editor().travel('undo')
    expect(editor().history.present[0].color).toBe(previous)
  })
  it('已提交的图形在轮询后保留撤销栈', async () => {
    await mount(); await draw()
    await vi.advanceTimersByTimeAsync(250)
    expect(editor().history.present).toHaveLength(1)
    expect(editor().history.past).toHaveLength(1)
  })
  it('停止通知关闭标注层，迟到的旧会话查询不能将其重新挂载', async () => {
    let resolve!: (v: typeof initial) => void
    api.recordingAnnotations.mockReturnValueOnce(new Promise(r => { resolve = r }))
    await mount()
    events.closed!({ payload: 7 })
    resolve(initial); await flush()
    expect(workspace()).toBeUndefined()
  })
})
