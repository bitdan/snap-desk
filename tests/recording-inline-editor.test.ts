import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import * as Vue from 'vue'
import * as icons from 'lucide-vue-next'
import * as editing from '../src/lib/recording-editor'

const api = { prepareRecordingEditor: vi.fn(), closeRecordingEditor: vi.fn(), trimRecording: vi.fn(), cancelRecordingTrim: vi.fn() }
const source = compileScript(parse(readFileSync('src/components/RecordingEditor.vue', 'utf8')).descriptor, { id: 'recording-inline-test', inlineTemplate: true }).content
const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const module = { exports: {} as { default: Vue.DefineComponent } }
new Function('require', 'exports', code)((name: string) => {
  if (name === 'vue') return Vue
  if (name === 'lucide-vue-next') return icons
  if (name === '@/lib/recording-editor') return editing
  if (name === '@/lib/api') return { api }
  if (name === '@/lib/i18n') return { t: (value: string) => value, te: (value: string) => value }
  if (name === '@tauri-apps/api/core') return { convertFileSrc: (token: string) => `recording://${token}` }
  if (name === '@tauri-apps/api/window') return { getCurrentWindow: () => ({ startDragging: vi.fn() }) }
  throw new Error(`Unexpected editor import: ${name}`)
}, module.exports)
const RecordingEditor = module.exports.default

// 内存媒体宿主执行真实组件逻辑；不读取媒体文件、不创建浏览器或原生窗口。
class Node {
  parent: Node | null = null
  children: Node[] = []
  props: Record<string, any> = {}
  text = ''
  currentTime = 0
  ended = false
  muted = false
  playbackRate = 1
  value: unknown
  selectedIndex = -1
  get options() { return this.children.filter(child => child.type === 'option') }
  focus = vi.fn()
  load = vi.fn()
  pause = vi.fn(() => this.props.onPause?.())
  play = vi.fn(async () => { this.props.onPlay?.() })
  removeAttribute = vi.fn((key: string) => { delete this.props[key] })
  addEventListener = vi.fn()
  constructor(public type: string) {}
  closest(selector: string): Node | null { return selector.split(',').map(value => value.trim()).includes(this.type) ? this : this.parent?.closest(selector) ?? null }
}
const renderer = Vue.createRenderer<Node, Node>({
  createElement: type => new Node(type), createText: text => Object.assign(new Node('#text'), { text }), createComment: type => new Node(type),
  setText: (node, text) => { node.text = text }, setElementText: (node, text) => { node.text = text },
  parentNode: node => node.parent, nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index >= 0) parent.children.splice(index, 0, node); else parent.children.push(node)
  },
  remove(node) { node.parent?.children.splice(node.parent.children.indexOf(node), 1); node.parent = null },
  patchProp(node, key, _old, value) { node.props[key] = value; if (key === 'value') node.value = value },
})
let root: Node
let close: ReturnType<typeof vi.fn>
function find(predicate: (node: Node) => boolean, parent = root): Node | undefined {
  if (predicate(parent)) return parent
  for (const child of parent.children) { const found = find(predicate, child); if (found) return found }
}
function control(label: string) { return find(node => node.props['aria-label'] === label)! }
const flush = async () => { for (let i = 0; i < 5; i++) { await Promise.resolve(); await Vue.nextTick() } }
async function mount(format = 'mp4') {
  api.prepareRecordingEditor.mockResolvedValue({ token: 9, format, durationMs: 3000, previewAvailable: true })
  renderer.render(Vue.h(RecordingEditor, { resultId: 42, disabled: false, floating: true, frameRate: 30, onClose: close,
    layout: { x: -1000, y: 100, width: 800, height: 600, video: { left: 5, top: 0, width: 95, height: 75 }, toolbar: { left: 0, top: 80, width: 100, height: 20 } },
  }), root)
  await flush()
  const media = find(node => node.type === 'video')
  media?.props.onLoadedmetadata(); await flush()
  return media
}
beforeEach(() => {
  vi.resetAllMocks(); root = new Node('root'); close = vi.fn()
  api.closeRecordingEditor.mockResolvedValue(undefined); api.trimRecording.mockResolvedValue(null)
})
afterEach(() => renderer.render(null, root))

describe('原选区内录屏编辑', () => {
  it('uses native placement, commits handle drags once, supports undo and exports the selected interval', async () => {
    await mount()
    expect(control('录屏画面，空格播放，方向键逐帧定位').props.style).toMatchObject({ left: '5%', height: '75%' })
    const handle = control('裁剪开始端点')
    handle.props.onInput({ target: { value: '1000' } }); handle.props.onInput({ target: { value: '1500' } }); handle.props.onChange()
    await flush()
    control('另存所选片段').props.onClick(); await flush()
    expect(api.trimRecording).toHaveBeenCalledWith(9, { startMs: 1500, endMs: 3000 })
    control('撤销裁剪').props.onClick(); await flush()
    expect(control('裁剪开始端点').props.value).toBe(0)
    control('重做裁剪').props.onClick(); await flush()
    expect(control('裁剪开始端点').props.value).toBe(1500)
  })
  it('loops the selected clip and releases media and native token when closed', async () => {
    const media = (await mount())!
    control('循环所选片段').props.onClick()
    await control('播放所选片段').props.onClick(); await flush()
    expect(media.play).toHaveBeenCalledOnce()
    media.currentTime = 3; media.ended = true; media.props.onTimeupdate()
    await media.props.onEnded(); await flush()
    expect(media.currentTime).toBe(0)
    expect(media.play).toHaveBeenCalledTimes(2)
    control('静音预览（不改变导出声音）').props.onClick(); expect(media.muted).toBe(true)
    control('关闭预览，保留已保存录屏').props.onClick(); expect(close).toHaveBeenCalledOnce()
    renderer.render(null, root); await flush()
    expect(media.pause).toHaveBeenCalled()
    expect(media.removeAttribute).toHaveBeenCalledWith('src')
    expect(media.load).toHaveBeenCalledOnce()
    expect(api.closeRecordingEditor).toHaveBeenCalledWith(9)
  })
  it('keeps GIF trim handles usable without pretending to provide frame-controlled GIF playback', async () => {
    expect(await mount('gif')).toBeUndefined()
    expect(control('播放所选片段').props.disabled).toBe(true)
    const handle = control('裁剪结束端点')
    expect(handle.props.disabled).toBe(false)
    handle.props.onInput({ target: { value: '1234' } }); handle.props.onChange(); await flush()
    control('另存所选片段').props.onClick(); await flush()
    expect(api.trimRecording).toHaveBeenCalledWith(9, { startMs: 0, endMs: 1230 })
  })
})
