import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { compile } from '@vue/compiler-dom'
import type { AnnotationEditor } from '../src/lib/use-annotations'

const api = vi.hoisted(() => ({ recordingAnnotations: vi.fn(), currentRecording: vi.fn(), updateRecordingAnnotations: vi.fn(), setRecordingAnnotationMode: vi.fn(), controlRecording: vi.fn() }))
const events = vi.hoisted(() => ({ closed: undefined as undefined | ((event: { payload: number }) => void), off: vi.fn() }))
vi.mock('../src/lib/api', () => ({ api }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_name: string, cb: typeof events.closed) => { events.closed = cb; return events.off } }))
vi.mock('../src/lib/toolbar-layout', () => ({ useFloatingToolbar: () => ({ style: {}, start: vi.fn(), reset: vi.fn() }), annotationToolbarPosition: () => ({}) }))
vi.mock('../src/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h, onMounted } = await import('vue')
  return { default: defineComponent({ props: ['editor', 'toolbarVisible', 'defaultExpanded'], emits: ['toolbarElement', 'tool'], setup: (props, { slots, emit }) => {
    onMounted(() => emit('toolbarElement', { getBoundingClientRect: () => ({ x: 100, y: 600, left: 100, top: 600, width: 800, height: 100 }) }))
    return () => h('workspace', { editor: props.editor, toolbarVisible: props.toolbarVisible, defaultExpanded: props.defaultExpanded, onChoose: () => emit('tool', 'arrow') }, props.toolbarVisible ? slots.actions?.() : [])
  } }) }
})
import Overlay from '../src/components/RecordingAnnotations.vue'
const { descriptor } = parse(readFileSync('src/components/RecordingAnnotations.vue', 'utf8'))
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
  it('返回桌面后原工具栏仍可操作，直接选择工具恢复绘制', async () => {
    await mount()
    const finish = find(n => n.type === 'button' && n.children.some(c => c.text.includes('继续操作桌面')))
    expect(finish).toBeDefined()
    ;(finish!.props.onClick as () => void)(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenCalledWith(7, false, { x: 100, y: 600, width: 800, height: 100 })
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
    const finish = find(n => n.type === 'button' && n.children.some(c => c.text.includes('继续操作桌面')))!
    ;(finish.props.onClick as () => void)(); await flush()
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
