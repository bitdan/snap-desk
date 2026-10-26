import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { createRenderer, h, nextTick, provide, ssrContextKey } from 'vue'
import { readFileSync } from 'node:fs'
import { parse, compileScript } from '@vue/compiler-sfc'
import { compile } from '@vue/compiler-dom'
import type { RecordingView } from '../src/lib/recording'

const api = vi.hoisted(() => ({ currentRecording: vi.fn(), latestRecordingResult: vi.fn(), controlRecording: vi.fn(), selectRecordingArea: vi.fn(), setRecordingAnnotationMode: vi.fn() }))
const closed = vi.hoisted(() => ({ subscribe: vi.fn(), callback: undefined as undefined | ((id: number) => void), off: vi.fn() }))
vi.mock('../src/lib/api', () => ({ api, onRecordingPreviewClosed: closed.subscribe }))
vi.mock('../src/components/RecordingEditor.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ props: { resultId: Number, disabled: Boolean, floating: Boolean, layout: Object, frameRate: Number }, emits: ['close', 'rerecord'],
    setup: (props, { emit }) => () => h('div', { 'data-editor-id': props.resultId, 'data-floating': props.floating, onClick: () => emit('close') }, [h('button', { 'data-rerecord': true, onClick: () => emit('rerecord') })]) }) }
})
import RecordingControls from '../src/components/RecordingControls.vue'
// Vitest 的 Node 管线保留真实 setup，模板另编译为客户端 render 供内存宿主执行。
const { descriptor } = parse(readFileSync(new URL('../src/components/RecordingControls.vue', import.meta.url), 'utf8'))
const bindings = compileScript(descriptor, { id: 'recording-controls-test' }).bindings
Object.assign(RecordingControls, { render: new Function('Vue', compile(descriptor.template!.content, { mode: 'function', prefixIdentifiers: true, bindingMetadata: bindings }).code)(Vue) })
const Wrapper = { setup() { provide(ssrContextKey, { modules: new Set() }); return () => h(RecordingControls) } }


// 使用内存渲染器测试实际组件状态切换，不启动浏览器或原生录屏。
interface Node { type: string; props: Record<string, unknown>; children: Node[]; parent: Node | null; text: string }
const node = (type: string): Node => ({ type, props: {}, children: [], parent: null, text: '' })
const renderer = createRenderer<Node, Node>({
  createElement: node, createText: text => ({ ...node('#text'), text }), createComment: node,
  setText: (node, text) => { node.text = text }, setElementText: (node, text) => { node.text = text; node.children = [] },
  parentNode: node => node.parent, nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  patchProp: (node, key, _old, value) => { node.props[key] = value },
  insert(child, parent, anchor) {
    if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1)
    child.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index < 0) parent.children.push(child); else parent.children.splice(index, 0, child)
  },
  remove(child) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null },
})
const root = node('root')
function find(predicate: (node: Node) => boolean, current = root): Node | undefined {
  if (predicate(current)) return current
  for (const child of current.children) { const found = find(predicate, child); if (found) return found }
}
function view(phase: RecordingView['phase'], id = 7): RecordingView {
  return { id, revision: 1, phase, seconds: 3, frames: 90, targetFps: 30, systemAudio: false, microphone: false,
    droppedFrames: 0, encodingMillis: 0, targetBitrate: 8000000, encoderKind: 'software', encoderFallback: null,
    countdownRemaining: 0, message: '', path: phase === 'completed' ? 'D:/video.mp4' : null,
    stats: { throttledSamples: 0, queuedFrames: 0, capturePolls: 0, capturedFrames: 90, captureMillis: 0, captureCopyBytes: 0, compositionCopyBytes: 0,
      composedFrames: 90, compositionMillis: 0, queueDroppedFrames: 0, staleQueuedFrames: 0, nv12OutputBytes: 0,
      recentFps: 30, rejectedCaptureFrames: 0, rejectedComposedFrames: 0 } }
}
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
async function mount() { renderer.render(h(Wrapper), root); await flush() }
function editor() { return find(node => node.props['data-editor-id'] !== undefined) }

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  api.currentRecording.mockResolvedValue(view('completed'))
  api.latestRecordingResult.mockResolvedValue({ id: 42, path: 'D:/video.mp4', filename: 'video.mp4' })
  api.controlRecording.mockResolvedValue(undefined)
  api.selectRecordingArea.mockResolvedValue(undefined)
  closed.subscribe.mockImplementation(async (callback: (id: number) => void) => { closed.callback = callback; return closed.off })
})
afterEach(() => { renderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers() })

describe('录屏控制条直接进入回放编辑', () => {
  it.each(['recording', 'paused'] as const)('%s 时可打开标注，保持当前录制状态', async phase => {
    api.currentRecording.mockResolvedValue(view(phase))
    await mount()
    const button = find(node => node.props['aria-label'] === '录制中标注')!
    expect(button.props.disabled).toBe(false)
    ;(button.props.onClick as () => void)()
    await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenCalledWith(7, true)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('打开标注失败显示原因并恢复按钮', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    api.setRecordingAnnotationMode.mockRejectedValueOnce(new Error('浮层不可用'))
    await mount()
    ;(find(node => node.props['aria-label'] === '录制中标注')!.props.onClick as () => void)()
    await flush()
    expect(find(node => node.props.role === 'alert')?.text).toContain('浮层不可用')
    expect(find(node => node.props['aria-label'] === '录制中标注')?.props.disabled).toBe(false)
  })
  it('完成后直接打开成品编辑器，轮询不重复创建编辑会话；关闭后不重新弹出', async () => {
    await mount()
    expect(editor()?.props['data-editor-id']).toBe(42)
    expect(editor()?.props['data-floating']).toBe(true)
    await vi.advanceTimersByTimeAsync(1000)
    expect(api.latestRecordingResult).toHaveBeenCalledTimes(1)
    ;(editor()!.props.onClick as () => void)()
    await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(7, 'cancel')
    expect(editor()).toBeUndefined()
    await vi.advanceTimersByTimeAsync(1000)
    expect(editor()).toBeUndefined()
    expect(api.latestRecordingResult).toHaveBeenCalledTimes(1)
  })
  it('再次录屏卸载旧编辑器并恢复开始按钮', async () => {
    await mount()
    api.currentRecording.mockResolvedValue(view('ready', 8))
    await vi.advanceTimersByTimeAsync(250)
    expect(editor()).toBeUndefined()
    const start = find(node => node.props['aria-label'] === '开始录制')!
    expect(start).toBeDefined()
    ;(start.props.onClick as () => void)()
    await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(8, 'start')
  })
  it('重新录制先关闭原选区回放，再进入框选，不打开主窗口', async () => {
    await mount()
    ;(find(node => node.props['data-rerecord'] === true)!.props.onClick as () => void)()
    await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(7, 'cancel')
    expect(api.selectRecordingArea).toHaveBeenCalledOnce()
    expect(api.controlRecording.mock.invocationCallOrder[0]).toBeLessThan(api.selectRecordingArea.mock.invocationCallOrder[0])
    expect(editor()).toBeUndefined()
  })
  it('原生关闭通知卸载播放器，旧会话关闭不会影响新会话', async () => {
    await mount()
    closed.callback!(6); await flush(); expect(editor()).toBeDefined()
    closed.callback!(7); await flush(); expect(editor()).toBeUndefined()
    await vi.advanceTimersByTimeAsync(1000)
    expect(editor()).toBeUndefined()
  })
  it('重新框选失败恢复原选区编辑，原片仍可操作', async () => {
    await mount()
    api.selectRecordingArea.mockRejectedValueOnce(new Error('截图正在进行中'))
    ;(find(node => node.props['data-rerecord'] === true)!.props.onClick as () => void)()
    await flush()
    expect(editor()?.props['data-editor-id']).toBe(42)
    expect(find(node => node.props.role === 'alert')?.text).toContain('截图正在进行中')
    closed.callback!(7); await flush() // 本地关闭的迟到通知不能关闭刚恢复的播放器。
    expect(editor()).toBeDefined()
    closed.callback!(7); await flush() // 后续真正的原生关闭仍然有效。
    expect(editor()).toBeUndefined()
  })
  it('关闭通知先于结果查询返回时不重新挂载媒体', async () => {
    let finish!: (value: unknown) => void
    api.latestRecordingResult.mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
    await mount()
    closed.callback!(7)
    finish({ id: 42, path: 'D:/video.mp4', filename: 'video.mp4' })
    await flush()
    expect(editor()).toBeUndefined()
  })
  it('结果已更新时拒绝打开错误录像，并允许用户重试', async () => {
    api.latestRecordingResult.mockResolvedValueOnce({ id: 41, path: 'D:/old.mp4', filename: 'old.mp4' })
    await mount()
    expect(editor()).toBeUndefined()
    expect(find(node => node.props.role === 'alert')?.text).toContain('结果已更新')
    const retry = find(node => node.type === 'button' && node.text === '重新读取录屏')!
    ;(retry.props.onClick as () => void)()
    await flush()
    expect(editor()?.props['data-editor-id']).toBe(42)
  })
})
