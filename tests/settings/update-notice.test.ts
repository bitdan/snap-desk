vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import type { AppUpdateView } from '../../src/features/settings/types'

const api = vi.hoisted(() => ({ appUpdateStatus: vi.fn(), downloadAppUpdate: vi.fn(), installAppUpdate: vi.fn() }))
const events = vi.hoisted(() => ({ receive: (_view: AppUpdateView) => {}, off: vi.fn(), open: () => {}, offRequest: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/features/settings/events', () => ({ onUpdateWindowRequested: (fn: () => void) => { events.open = fn; return events.offRequest }, onAppUpdate: async (fn: typeof events.receive) => { events.receive = fn; return events.off } }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import UpdateNotice from '../../src/features/settings/components/UpdateNotice.vue'
const Component = clientComponent(UpdateNotice, new URL('../../src/features/settings/components/UpdateNotice.vue', import.meta.url))
let root: MemoryNode, native: AppUpdateView
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const action = (label: string) => findNode(root, node => node.type === 'button' && text(node) === label)
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent(); action('有更新')!.props.onClick(); await flushComponent() }
beforeEach(() => {
  vi.clearAllMocks(); vi.useFakeTimers()
  root = new MemoryNode('root')
  native = { currentVersion: '0.2.0', availableVersion: '0.2.1', notes: '不展示的更新日志', packageKind: 'nsis',
    supported: true, configured: true, phase: 'available', downloaded: 0, total: null, error: null }
  api.appUpdateStatus.mockImplementation(async () => ({ ...native }))
})
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers() })

describe('独立更新窗口', () => {
  it('主页面只显示有更新，点击后才展示下载控件', async () => {
    memoryRenderer.render(h(Component), root); await flushComponent()
    expect(action('有更新')).toBeDefined()
    expect(action('下载更新')).toBeUndefined()
    action('有更新')!.props.onClick(); await flushComponent()
    expect(findNode(root, node => node.props['data-window'] === '应用更新')).toBeDefined()
    expect(action('下载更新')).toBeDefined()
  })
  it('可直接下载、查看进度，完成后由用户安装', async () => {
    let finish!: () => void
    api.downloadAppUpdate.mockImplementation(() => {
      native.phase = 'downloading'; native.downloaded = 40; native.total = 100
      return new Promise(resolve => { finish = () => { native.phase = 'ready'; resolve({ ...native }) } })
    })
    await mount()
    expect(text(root)).toContain('发现新版本 · v0.2.1')
    expect(text(root)).not.toContain(native.notes)
    const download = action('下载更新')!.props.onClick()
    await vi.advanceTimersByTimeAsync(1000); await flushComponent()
    expect(text(root)).toContain('40%')
    expect(action('退出并安装')).toBeUndefined()
    finish(); await download; await flushComponent()
    expect(action('退出并安装')).toBeDefined()
    expect(api.installAppUpdate).not.toHaveBeenCalled()
    await action('退出并安装')!.props.onClick()
    expect(api.installAppUpdate).toHaveBeenCalledOnce()
  })
  it('关闭窗口后保留入口，重新打开读取同一后台状态', async () => {
    await mount()
    findNode(root, node => node.props['aria-label'] === '关闭')!.props.onClick()
    await flushComponent()
    expect(action('下载更新')).toBeUndefined()
    native.availableVersion = '0.2.2'; events.receive({ ...native }); events.open(); await flushComponent()
    expect(action('下载更新')).toBeDefined()
    expect(text(root)).toContain('v0.2.2')
    memoryRenderer.render(null, root)
    expect(events.off).toHaveBeenCalledOnce()
    expect(events.offRequest).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('安装被录屏占用拒绝时保留安装入口', async () => {
    native.phase = 'ready'
    api.installAppUpdate.mockRejectedValueOnce(new Error('请先结束录屏'))
    await mount(); await action('退出并安装')!.props.onClick(); await flushComponent()
    expect(text(root)).toContain('请先结束录屏')
    expect(action('退出并安装')!.props.disabled).toBe(false)
  })
  it('下载失败显示原因并保留重试入口', async () => {
    api.downloadAppUpdate.mockImplementation(async () => {
      native.phase = 'error'; native.error = '更新签名校验失败'
      throw new Error(native.error)
    })
    await mount(); await action('下载更新')!.props.onClick(); await flushComponent()
    expect(text(root)).toContain('更新签名校验失败')
    expect(action('下载更新')!.props.disabled).toBe(false)
    expect(action('退出并安装')).toBeUndefined()
  })
})
