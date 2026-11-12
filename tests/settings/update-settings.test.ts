import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'
import type { AppUpdateView } from '../../src/features/settings/types'

const openWindow = vi.hoisted(() => vi.fn())
const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), appUpdateStatus: vi.fn(), checkAppUpdate: vi.fn(), downloadAppUpdate: vi.fn(), installAppUpdate: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/features/settings/events', () => ({ requestUpdateWindow: openWindow, onSettingsImported: vi.fn(async () => () => {}) }))
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
  native = { currentVersion: '0.2.0', availableVersion: null, notes: null, packageKind: 'nsis', supported: true,
    configured: true, phase: 'idle', downloaded: 0, total: null, error: null }
  api.status.mockResolvedValue({ settings: defaultSettings() })
  api.appUpdateStatus.mockImplementation(async () => ({ ...native }))
  api.checkAppUpdate.mockImplementation(async () => ({ ...native }))
  api.saveSettings.mockImplementation(async settings => settings)
})
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers() })

describe('在线更新设置', () => {
  it('只显示简洁更新信息，不展示更新日志', async () => {
    native.availableVersion = '0.2.1'; native.phase = 'available'; native.notes = '不应显示的更新日志'
    await mount()
    expect(text(root)).toContain('有更新')
    expect(action('有更新')).toBeDefined()
    expect(text(root)).not.toContain(native.notes)
  })
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
  it('测试版构建固定接收测试版，开关显示开启且不可切换', async () => {
    native.currentVersion = '0.2.0-beta.2'
    await mount()
    expect(toggle().props['aria-checked']).toBe(true)
    expect(toggle().props.disabled).toBe(true)
    await toggle().props.onClick?.(); await flushComponent()
    expect(api.saveSettings).not.toHaveBeenCalled()
  })
  it('保存失败保留已生效渠道并显示重试原因', async () => {
    api.saveSettings.mockRejectedValueOnce(new Error('无法保存设置'))
    await mount(); await toggle().props.onClick(); await flushComponent()
    expect(toggle().props['aria-checked']).toBe(false)
    expect(text(root)).toContain('无法保存设置')
    expect(api.checkAppUpdate).not.toHaveBeenCalled()
  })
  it('有更新只打开独立窗口，由窗口负责下载与安装', async () => {
    native.availableVersion = '0.2.1'; native.phase = 'available'
    await mount(); await action('有更新').props.onClick(); await flushComponent()
    expect(openWindow).toHaveBeenCalledOnce()
    expect(api.downloadAppUpdate).not.toHaveBeenCalled()
    expect(api.installAppUpdate).not.toHaveBeenCalled()
  })
  it('后台下载和下载完成仍可重开独立窗口', async () => {
    vi.useFakeTimers()
    native.availableVersion = '0.2.1'; native.phase = 'downloading'
    await mount()
    expect(action('有更新').props.disabled).toBe(false)
    expect(toggle().props.disabled).toBe(true)
    await action('有更新').props.onClick()
    native.phase = 'ready'
    await vi.advanceTimersByTimeAsync(1000); await flushComponent()
    await action('有更新').props.onClick()
    expect(openWindow).toHaveBeenCalledTimes(2)
    expect(action('退出并安装')).toBeUndefined()
  })
  it('未配置签名或开发版本禁用更新操作', async () => {
    vi.useFakeTimers()
    native.configured = false
    await mount()
    expect(action('检查更新').props.disabled).toBe(true)
    expect(text(root)).toContain('此版本未配置更新签名公钥。')
    native.supported = false
    await vi.advanceTimersByTimeAsync(1000); await flushComponent()
    expect(action('检查更新').props.disabled).toBe(true)
    expect(text(root)).not.toContain('Windows x64 发布版本')
    expect(text(root)).not.toContain('此版本未配置更新签名公钥。')
  })
})
