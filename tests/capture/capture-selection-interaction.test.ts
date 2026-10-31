import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ current: vi.fn(), currentScroll: vi.fn(), recordingCapabilities: vi.fn(), startScroll: vi.fn(), startRecording: vi.fn() }))
const captureEvents = vi.hoisted(() => ({ frame: (_value: unknown) => {} }))
vi.mock('../../src/services/api', () => ({ api,
  onFrame: async (callback: typeof captureEvents.frame) => { captureEvents.frame = callback; return () => { captureEvents.frame = () => {} } },
  onCaptureClosed: async () => () => {}, onCapturePin: async () => () => {}, onScroll: async () => () => {},
}))
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: () => 'capture-image', isTauri: () => false }))
vi.mock('../../src/features/annotations/components/AnnotationWorkspace.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('workspace', attrs, [slots.actions?.(), slots.footer?.()]) }) }
})
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/features/recording/components/RecordingSetupMenu.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/recording/components/RecordingSourceSelect.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/recording/components/RecordingAnnotations.vue', () => ({ default: { render: () => null } }))
vi.mock('../../src/features/scroll/components/ScrollCaptureIcon.vue', () => ({ default: { render: () => null } }))
import Capture from '../../src/features/capture/components/CaptureOverlay.vue'
const Component = clientComponent(Capture, new URL('../../src/features/capture/components/CaptureOverlay.vue', import.meta.url))
let root: MemoryNode
const workspace = () => findNode(root, node => node.type === 'workspace')!
const overlay = () => findNode(root, node => node.props.class === 'capture-overlay')!
const recordingButton = () => findNode(root, node => node.props['aria-label'] === '展开录屏工具栏')!
const recordingConfirm = () => findNode(root, node => node.props.class === 'recording-confirm')!
const nodeText = (node: MemoryNode): string => node.text + node.children.map(nodeText).join('')
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
  api.startScroll.mockResolvedValue(undefined)
  memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent()
  overlay().props.onPointerdown(event(100, 100))
  overlay().props.onPointermoveCapture(event(500, 400))
  overlay().props.onPointerup(event(500, 400)); await flushComponent()
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it('普通截图和开始长截图均不查询录屏能力', async () => {
  expect(api.recordingCapabilities).not.toHaveBeenCalled()
  expect(recordingButton().props.disabled).toBe(false)
  await findNode(root, node => node.props['aria-label'] === '开始滚动截图')!.props.onClick()
  await flushComponent()
  expect(api.startScroll).toHaveBeenCalledExactlyOnceWith(1, { x: 100, y: 100, width: 400, height: 300 }, false, 'white')
  expect(api.recordingCapabilities).not.toHaveBeenCalled()
})

it('首次进入录屏才查询能力，等待时立即展示准备栏并禁止确认', async () => {
  let complete!: (value: { available: boolean; formats: string[] }) => void
  api.recordingCapabilities.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  recordingButton().props.onClick(); await flushComponent()
  expect(api.recordingCapabilities).toHaveBeenCalledOnce()
  expect(findNode(root, node => node.props['aria-label'] === '录屏准备工具栏')).toBeDefined()
  expect(findNode(root, node => node.props.role === 'status' && nodeText(node) === '正在检查录屏能力…')).toBeDefined()
  expect(recordingConfirm().props.disabled).toBe(true)
  await recordingConfirm().props.onClick()
  expect(api.startRecording).not.toHaveBeenCalled()
  complete({ available: true, formats: ['mp4'] }); await flushComponent()
  expect(recordingConfirm().props.disabled).toBe(false)
  expect(findNode(root, node => node.props.role === 'status' && nodeText(node) === '正在检查录屏能力…')).toBeUndefined()
})

it('收到录屏意图的截图会话自动读取能力，未框选时也展示检查状态', async () => {
  let complete!: (value: { available: boolean; formats: string[] }) => void
  api.recordingCapabilities.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  captureEvents.frame({ ...await api.current(), id: 2, recording: true }); await flushComponent()
  expect(api.recordingCapabilities).toHaveBeenCalledOnce()
  expect(findNode(root, node => node.props.role === 'status' && nodeText(node) === '正在检查录屏能力…')).toBeDefined()
  complete({ available: true, formats: ['mp4'] }); await flushComponent()
  expect(api.startRecording).not.toHaveBeenCalled()
})

it('能力查询失败后展示原因并可重试，成功后保留当前范围', async () => {
  api.recordingCapabilities.mockRejectedValueOnce(new Error('设备检查失败'))
  recordingButton().props.onClick(); await flushComponent()
  expect(recordingConfirm().props.disabled).toBe(true)
  expect(findNode(root, node => node.props.role === 'alert' && nodeText(node).includes('设备检查失败'))).toBeDefined()
  await findNode(root, node => node.props['aria-label'] === '重新读取')!.props.onClick(); await flushComponent()
  expect(api.recordingCapabilities).toHaveBeenCalledTimes(2)
  expect(recordingConfirm().props.disabled).toBe(false)
  expect(workspace().props.bounds).toEqual({ x: 100, y: 100, width: 400, height: 300 })
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
})

it('能力不支持也能进入录屏查看原因并重试，迟到回包不切换普通截图模式', async () => {
  api.recordingCapabilities.mockResolvedValueOnce({ available: false, formats: [], reason: '当前设备不支持' })
  recordingButton().props.onClick(); await flushComponent()
  expect(recordingConfirm().props.disabled).toBe(true)
  expect(findNode(root, node => node.props.role === 'alert' && nodeText(node).includes('当前设备不支持'))).toBeDefined()
  let complete!: (value: { available: boolean; formats: string[] }) => void
  api.recordingCapabilities.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  findNode(root, node => node.props['aria-label'] === '重新读取')!.props.onClick(); await flushComponent()
  captureEvents.frame({ ...await api.current(), id: 2, recording: false }); await flushComponent()
  complete({ available: true, formats: ['mp4'] }); await flushComponent()
  expect(recordingButton().props['aria-pressed']).toBe(false)
  expect(findNode(root, node => node.props['aria-label'] === '录屏准备工具栏')).toBeUndefined()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
})

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
