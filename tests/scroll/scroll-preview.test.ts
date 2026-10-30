import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, MemoryNode, memoryRenderer, findNode, flushComponent } from '../helpers/component-host'
import type { ScrollView } from '../../src/features/scroll/types'

const mocks = vi.hoisted(() => ({ currentScroll: vi.fn(), controlScroll: vi.fn(), onScroll: vi.fn(), onCaptureClosed: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api: mocks, onScroll: mocks.onScroll, onCaptureClosed: mocks.onCaptureClosed }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }))
import ScrollPreview from '../../src/features/scroll/components/ScrollPreview.vue'
const Component = clientComponent(ScrollPreview, new URL('../../src/features/scroll/components/ScrollPreview.vue', import.meta.url))
let root: MemoryNode
let receive: (view: ScrollView) => void
let close: (id: number) => void
const view: ScrollView = { id: 7, revision: 1, collecting: true, archivePath: null,
  width: 160, height: 320, frames: 2, message: '等待页面稳定', preview: 'data:image/png;base64,trusted', frameHeight: 160,
  canvasBounds: { x: 0, y: 0, width: 160, height: 320 }, viewportRect: { x: 0, y: 160, width: 160, height: 160 },
  coveragePercent: 100, hasHoles: false, direction: 'down', status: 'sampling' }
beforeEach(() => {
  vi.resetAllMocks()
  root = new MemoryNode('root')
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: vi.fn() })
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('location', { search: '?window=scroll-controls' })
  mocks.currentScroll.mockResolvedValue(view)
  mocks.controlScroll.mockResolvedValue(undefined)
  mocks.onScroll.mockImplementation(async callback => { receive = callback; return vi.fn() })
  mocks.onCaptureClosed.mockImplementation(async callback => { close = callback; return vi.fn() })
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent() }

describe('滚动采样状态与控制', () => {
  it.each([['sampling', '请停一下'], ['no_progress', '暂无新增'], ['waiting', '请稍候']] as const)('控制条显示 %s 且仍能完成', async (status, label) => {
    mocks.currentScroll.mockResolvedValue({ ...view, status })
    await mount()
    expect(findNode(root, node => node.text === label)).toBeDefined()
    const save = findNode(root, node => node.type === 'button' && node.props.class === 'save')!
    save.props.onClick(); await flushComponent()
    expect(mocks.controlScroll).toHaveBeenCalledWith(7, false, false)
  })
  it('恢复时保存可信部分，迟到事件不能恢复已关闭会话', async () => {
    await mount()
    receive({ ...view, revision: 2, status: 'recovering', message: '请滚回重叠区' }); await flushComponent()
    const save = findNode(root, node => node.type === 'button' && node.props.class === 'save')!
    save.props.onClick(); await flushComponent()
    expect(mocks.controlScroll).toHaveBeenCalledWith(7, false, true)
    close(7); receive({ ...view, revision: 99 }); await flushComponent()
    expect(save.props.disabled).toBe(true)
  })
  it.each(['scroll-preview', 'scroll-controls'])('%s 收到 Esc 时取消当前会话且卸载时移除监听', async label => {
    vi.stubGlobal('location', { search: `?window=${label}` })
    await mount()
    const key = vi.mocked(window.addEventListener).mock.calls.find(([type]) => type === 'keydown')![1] as (event: KeyboardEvent) => void
    const event = { key: 'Escape', preventDefault: vi.fn() } as unknown as KeyboardEvent
    key(event); await flushComponent()
    expect(event.preventDefault).toHaveBeenCalledOnce()
    expect(mocks.controlScroll).toHaveBeenCalledWith(7, true, false)
    memoryRenderer.render(null, root)
    expect(window.removeEventListener).toHaveBeenCalledWith('keydown', key)
  })
  it('完成请求尚未返回时 Esc 仍可取消，旧失败不覆盖取消状态', async () => {
    let rejectFinish!: (error: Error) => void
    mocks.controlScroll.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectFinish = reject }))
    await mount()
    findNode(root, node => node.type === 'button' && node.props.class === 'save')!.props.onClick()
    await flushComponent()
    const key = vi.mocked(window.addEventListener).mock.calls.find(([type]) => type === 'keydown')![1] as (event: KeyboardEvent) => void
    key({ key: 'Escape', preventDefault: vi.fn() } as unknown as KeyboardEvent)
    await flushComponent()
    expect(mocks.controlScroll).toHaveBeenLastCalledWith(7, true, false)
    rejectFinish(new Error('过期的完成请求'))
    await flushComponent()
    expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
    expect(findNode(root, node => node.text === '正在取消…')).toBeDefined()
  })
  it('取消始终转发当前会话', async () => {
    await mount()
    findNode(root, node => node.type === 'button' && node.props.class === 'cancel')!.props.onClick()
    await flushComponent()
    expect(mocks.controlScroll).toHaveBeenCalledWith(7, true, false)
  })
  it('预览窗口接收状态提示但保留可信图像', async () => {
    vi.stubGlobal('location', { search: '?window=scroll-preview' })
    await mount()
    receive({ ...view, revision: 2, status: 'no_progress', message: '尚无新增内容' }); await flushComponent()
    expect(findNode(root, node => node.type === 'img')?.props.src).toBe(view.preview)
    expect(findNode(root, node => node.type === 'p' && node.props.title === '尚无新增内容')).toBeDefined()
  })
})
