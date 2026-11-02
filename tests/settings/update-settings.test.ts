import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'
import type { AppUpdateView } from '../../src/features/settings/types'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), appUpdateStatus: vi.fn(), checkAppUpdate: vi.fn(), downloadAppUpdate: vi.fn(), installAppUpdate: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/features/settings/events', () => ({ onSettingsImported: vi.fn(async () => () => {}) }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import UpdateSettings from '../../src/features/settings/components/UpdateSettings.vue'
const Component = clientComponent(UpdateSettings, new URL('../../src/features/settings/components/UpdateSettings.vue', import.meta.url))
let root: MemoryNode, native: AppUpdateView
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const action = (label: string) => findNode(root, node => node.type === 'button' && text(node) === label)!
const toggle = () => findNode(root, node => node.props.role === 'switch')!
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent() }
beforeEach(() => {
  vi.resetAllMocks()
  root = new MemoryNode('root')
  native = { currentVersion: '0.2.0-beta.2', availableVersion: null, notes: null, packageKind: 'nsis', supported: true,
    configured: true, phase: 'idle', downloaded: 0, total: null, error: null }
  api.status.mockResolvedValue({ settings: defaultSettings() })
  api.appUpdateStatus.mockImplementation(async () => ({ ...native }))
  api.checkAppUpdate.mockImplementation(async () => ({ ...native }))
  api.saveSettings.mockImplementation(async settings => settings)
})
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers() })

describe('在线更新设置', () => {
  it('默认稳定渠道，切换只保存更新偏好并立即重新检查', async () => {
    const settings = defaultSettings(); settings.delaySeconds = 5
    api.status.mockResolvedValue({ settings })
    await mount()
    expect(toggle().props['aria-checked']).toBe(false)
    await toggle().props.onClick(); await flushComponent()
    expect(api.saveSettings).toHaveBeenCalledWith({ ...settings, receivePrereleaseUpdates: true })
    expect(toggle().props['aria-checked']).toBe(true)
    expect(api.checkAppUpdate).toHaveBeenCalledOnce()
  })
  it('保存失败保留已生效渠道并显示重试原因', async () => {
    api.saveSettings.mockRejectedValueOnce(new Error('无法保存设置'))
    await mount(); await toggle().props.onClick(); await flushComponent()
    expect(toggle().props['aria-checked']).toBe(false)
    expect(text(root)).toContain('无法保存设置')
    expect(api.checkAppUpdate).not.toHaveBeenCalled()
  })
  it('验签失败不会出现安装按钮，允许重新下载', async () => {
    native.availableVersion = '0.2.0'; native.phase = 'available'
    api.downloadAppUpdate.mockImplementation(async () => {
      native.phase = 'error'; native.error = '更新签名校验失败'
      throw new Error(native.error)
    })
    await mount(); await action('下载更新').props.onClick(); await flushComponent()
    expect(action('退出并安装')).toBeUndefined()
    expect(action('下载更新').props.disabled).toBe(false)
    expect(text(root)).toContain('更新签名校验失败')
  })
  it('显示原生下载进度，下载完成后才允许显式安装', async () => {
    vi.useFakeTimers()
    native.availableVersion = '0.2.0'; native.phase = 'available'
    let finish!: () => void
    api.downloadAppUpdate.mockImplementation(() => {
      native.phase = 'downloading'; native.downloaded = 40; native.total = 100
      return new Promise(resolve => { finish = () => { native.phase = 'ready'; resolve({ ...native }) } })
    })
    await mount()
    const download = action('下载更新').props.onClick()
    await vi.advanceTimersByTimeAsync(1000); await flushComponent()
    expect(text(root)).toContain('40%')
    expect(toggle().props.disabled).toBe(true)
    expect(action('退出并安装')).toBeUndefined()
    finish(); await download; await flushComponent()
    expect(action('退出并安装')).toBeDefined()
    expect(api.installAppUpdate).not.toHaveBeenCalled()
    await action('退出并安装').props.onClick()
    expect(api.installAppUpdate).toHaveBeenCalledOnce()
  })
  it('安装因录屏被拒绝时保留重试按钮', async () => {
    native.availableVersion = '0.2.0'; native.phase = 'ready'
    api.installAppUpdate.mockRejectedValueOnce(new Error('请先结束录屏并等待视频保存完成。'))
    await mount(); await action('退出并安装').props.onClick(); await flushComponent()
    expect(text(root)).toContain('请先结束录屏')
    expect(action('退出并安装').props.disabled).toBe(false)
  })
  it('未配置签名或开发版本禁用更新操作', async () => {
    vi.useFakeTimers()
    native.configured = false
    await mount()
    expect(action('检查更新').props.disabled).toBe(true)
    expect(text(root)).toContain('此版本未配置更新签名公钥。')
    native.supported = false
    await vi.advanceTimersByTimeAsync(1000); await flushComponent()
    expect(text(root)).toContain('Windows x64 发布版本')
  })
})
