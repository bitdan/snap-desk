import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ current: vi.fn(), currentScroll: vi.fn(), recordingCapabilities: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api,
  onFrame: async () => () => {}, onCaptureClosed: async () => () => {}, onCapturePin: async () => () => {}, onScroll: async () => () => {},
}))
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: () => 'capture-image', isTauri: () => false }))
vi.mock('../../src/features/annotations/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('workspace', attrs, slots.actions?.()) }) }
})
vi.mock('../../src/features/recording/components/RecordingAnnotations.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/scroll/components/ScrollCaptureIcon.vue', () => ({ default: { render: () => null } }))
import Capture from '../../src/features/capture/components/CaptureOverlay.vue'
const Component = clientComponent(Capture, new URL('../../src/features/capture/components/CaptureOverlay.vue', import.meta.url))
let root: MemoryNode
const workspace = () => findNode(root, node => node.type === 'workspace')!
const overlay = () => findNode(root, node => node.props.class === 'capture-overlay')!
const event = (x: number, y: number) => ({ clientX: x, clientY: y, pointerId: 1, button: 0, shiftKey: true, currentTarget: overlay(), target: { closest: () => null }, stopPropagation: vi.fn() })
beforeEach(async () => {
  root = new MemoryNode('root'); vi.resetAllMocks()
  vi.stubGlobal('innerWidth', 1000); vi.stubGlobal('innerHeight', 800)
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('document', { documentElement: { classList: { toggle: vi.fn() } } })
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  Object.assign(MemoryNode.prototype, { setPointerCapture: vi.fn() })
  api.current.mockResolvedValue({ id: 1, width: 1000, height: 800, previewRevision: 1, originX: 0, originY: 0, displays: [], targets: [], recording: false })
  api.currentScroll.mockResolvedValue(null)
  api.recordingCapabilities.mockResolvedValue({ available: true, formats: ['mp4'] })
  memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent()
  overlay().props.onPointerdown(event(100, 100))
  overlay().props.onPointermoveCapture(event(500, 400))
  overlay().props.onPointerup(event(500, 400)); await flushComponent()
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('已有标注仍可直接拖动边框和控制点，保留原图坐标和撤销栈', async () => {
  const editor = workspace().props.editor
  workspace().props.onTool('rectangle')
  editor.begin({ x: 150, y: 150 }, 2); editor.drag({ x: 250, y: 250 }); await editor.finish(); await flushComponent()
  const shapes = structuredClone(JSON.parse(JSON.stringify(editor.history.present)))
  const history = editor.history.past.length
  expect(findNode(root, node => node.props['aria-label'] === '调整选区')).toBeUndefined()
  const edge = findNode(root, node => String(node.props.class).includes('selection-drag-edge'))!
  edge.props.onPointerdown(event(300, 100)); await flushComponent()
  expect(workspace().props['toolbar-visible']).toBe(false)
  overlay().props.onPointermoveCapture(event(350, 120)); overlay().props.onPointerup(event(350, 120)); await flushComponent()
  expect(workspace().props['toolbar-visible']).toBe(true)
  expect(workspace().props.bounds).toEqual({ x: 150, y: 120, width: 400, height: 300 })
  expect(editor.history.present).toEqual(shapes); expect(editor.history.past).toHaveLength(history)
  const handle = findNode(root, node => String(node.props.class).includes('resize-handle se'))!
  handle.props.onPointerdown(event(550, 420)); overlay().props.onPointermoveCapture(event(650, 520)); overlay().props.onPointerup(event(650, 520)); await flushComponent()
  expect(workspace().props.bounds).toEqual({ x: 150, y: 120, width: 500, height: 400 })
  expect(editor.history.present).toEqual(shapes)
})

it('绘制标注期间保留工具栏和定位；指针工具空白拖动可调整选区，取消恢复原范围', async () => {
  const editor = workspace().props.editor
  workspace().props.onTool('rectangle'); await flushComponent()
  const position = workspace().props['toolbar-style'][0]
  editor.begin({ x: 150, y: 150 }, 2); await flushComponent()
  expect(workspace().props['toolbar-visible']).toBe(true)
  expect(workspace().props['toolbar-style'][0]).toEqual(position)
  editor.drag({ x: 250, y: 250 }); await editor.finish(); await flushComponent()
  workspace().props.onTool('edit'); await flushComponent()
  workspace().props.onBackgroundPointerdown(event(300, 300)); overlay().props.onPointermoveCapture(event(400, 400)); await flushComponent()
  expect(workspace().props['toolbar-visible']).toBe(false)
  expect(workspace().props.bounds).toEqual({ x: 200, y: 200, width: 400, height: 300 })
  expect(workspace().props['toolbar-style'][0]).toEqual(position)
  overlay().props.onPointercancel({}); await flushComponent()
  expect(workspace().props['toolbar-visible']).toBe(true)
  expect(workspace().props.bounds).toEqual({ x: 100, y: 100, width: 400, height: 300 })
  expect(editor.history.present).toHaveLength(1)
})
