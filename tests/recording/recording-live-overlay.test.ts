import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { compile } from '@vue/compiler-dom'
import type { AnnotationEditor } from '../../src/features/annotations/use-annotations'

const api = vi.hoisted(() => ({ recordingAnnotations: vi.fn(), currentRecording: vi.fn(), updateRecordingAnnotations: vi.fn(), setRecordingAnnotationMode: vi.fn(), controlRecording: vi.fn() }))
const events = vi.hoisted(() => ({ closed: undefined as undefined | ((event: { payload: number }) => void), off: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_name: string, cb: typeof events.closed) => { events.closed = cb; return events.off } }))
vi.mock('../../src/features/annotations/toolbar-layout', () => ({ useFloatingToolbar: () => ({ style: {}, start: vi.fn(), reset: vi.fn() }), annotationToolbarPosition: () => ({}) }))
vi.mock('../../src/features/annotations/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h, onMounted } = await import('vue')
  return { default: defineComponent({ props: ['editor', 'toolbarVisible', 'defaultExpanded'], emits: ['toolbarElement', 'tool'], setup: (props, { slots, emit }) => {
    onMounted(() => emit('toolbarElement', { getBoundingClientRect: () => ({ x: 100, y: 600, left: 100, top: 600, width: 800, height: 100 }) }))
    return () => h('workspace', { editor: props.editor, toolbarVisible: props.toolbarVisible, defaultExpanded: props.defaultExpanded, onChoose: () => emit('tool', 'arrow') }, props.toolbarVisible ? slots.actions?.() : [])
  } }) }
})
import Overlay from '../../src/features/recording/components/RecordingAnnotations.vue'
const { descriptor } = parse(readFileSync('src/features/recording/components/RecordingAnnotations.vue', 'utf8'))
const bindings = compileScript(descriptor, { id: 'live-overlay-test' }).bindings
Object.assign(Overlay, { render: new Function('Vue', compile(descriptor.template!.content, { mode: 'function', prefixIdentifiers: true, bindingMetadata: bindings }).code)(Vue) })
interface Node { type: string; props: Record<string, any>; children: Node[]; parent: Node | null; text: string }
const node = (type: string): Node => ({ type, props: {}, children: [], parent: null, text: '' })
const renderer = Vue.createRenderer<Node, Node>({
  createElement: node, createText: text => ({ ...node('#text'), text }), createComment: node,
  setText: (n, text) => { n.text = text }, setElementText: (n, text) => { n.text = text },
  parentNode: n => n.parent, nextSibling: n => n.parent?.children[n.parent.children.indexOf(n) + 1] ?? null,
  patchProp: (n, key, _old, value) => { n.props[key] = value },
  insert(child, parent, anchor) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = parent; const i = anchor ? parent.children.indexOf(anchor) : -1; if (i < 0) parent.children.push(child); else parent.children.splice(i, 0, child) },
  remove(child) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null },
})
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
async function mount() {
  const Wrapper = { setup() { Vue.provide(Vue.ssrContextKey, { modules: new Set() }); return () => Vue.h(Overlay) } }
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
  it('Ready 不自动开始，空格依次开始、暂停、继续', async () => {
    api.currentRecording.mockResolvedValue({ id: 7, phase: 'ready', seconds: 0 })
    await mount()
    expect(api.controlRecording).not.toHaveBeenCalled()
    expect(find(n => n.type === 'button' && n.children.some(c => c.text.includes('开始录制')))).toBeDefined()
    for (const [phase, action] of [['recording', 'start'], ['paused', 'pause'], ['recording', 'resume']]) {
      api.currentRecording.mockResolvedValue({ id: 7, phase, seconds: 0 })
      const event = press(' '); await flush()
      expect(event.preventDefault).toHaveBeenCalled()
      expect(api.controlRecording).toHaveBeenLastCalledWith(7, action)
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
  it('不再显示继续操作桌面按钮，外部桌面模式仍可直接选择工具恢复绘制', async () => {
    api.recordingAnnotations.mockResolvedValueOnce({ ...initial, editing: false })
    await mount()
    expect(find(n => n.type === 'button' && n.children.some(c => c.text.includes('继续操作桌面')))).toBeUndefined()
    expect(workspace()?.props.toolbarVisible).toBe(true)
    expect(api.controlRecording).not.toHaveBeenCalled()
    ;(workspace()!.props.onChoose as () => void)(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenLastCalledWith(7, true)
    expect(editor().tool).toBe('arrow')
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
