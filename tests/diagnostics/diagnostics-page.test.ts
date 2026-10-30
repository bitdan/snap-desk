import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer, flushComponent } from '../helpers/component-host'
import type { DiagnosticsStatus, RecentLogs } from '../../src/features/diagnostics/diagnostics'

const api = vi.hoisted(() => ({ diagnosticsStatus: vi.fn(), diagnosticsRecentLogs: vi.fn(), diagnosticsSetLevel: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import DiagnosticsSettings from '../../src/features/diagnostics/components/DiagnosticsSettings.vue'
const Component = clientComponent(DiagnosticsSettings, new URL('../../src/features/diagnostics/components/DiagnosticsSettings.vue', import.meta.url))
let root: MemoryNode
let status: DiagnosticsStatus
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const control = (label: string) => findNode(root, node => node.props['aria-label'] === label)!
const recent: RecentLogs = { entries: [{ timestamp: '2026-10-30', level: 'INFO', module: 'app', message: 'event=app.starting' }], scannedBytes: 10, omittedLines: 0, truncated: false }
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks(); vi.stubGlobal('document', { visibilityState: 'visible' })
  root = new MemoryNode('root')
  status = { level: 'INFO', directory: 'C:/logs', error: null, totalBytes: 10, archives: 0, recentErrors: 0, exportSupported: true }
  api.diagnosticsStatus.mockResolvedValue(status)
  api.diagnosticsRecentLogs.mockResolvedValue(recent)
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent() }
async function choose(label: string, value: string) { control(label).props['onUpdate:modelValue'](value); await flushComponent() }

describe('日志页面', () => {
  it('记录级别以后端返回为准，写入失败保留已生效选项', async () => {
    await mount()
    api.diagnosticsSetLevel.mockResolvedValueOnce({ ...status, level: 'DEBUG' })
    await choose('记录级别', 'DEBUG')
    expect(api.diagnosticsSetLevel).toHaveBeenCalledWith('DEBUG')
    expect(control('记录级别').value).toBe('DEBUG')
    api.diagnosticsSetLevel.mockRejectedValueOnce(new Error('无法保存'))
    await choose('记录级别', 'ERROR')
    expect(control('记录级别').value).toBe('DEBUG')
    expect(text(findNode(root, node => node.props.role === 'alert')!)).toContain('无法保存')
  })
  it('显示筛选只发送日志查询，不改变记录级别', async () => {
    await mount()
    await choose('显示级别', 'WARN')
    expect(api.diagnosticsRecentLogs).toHaveBeenLastCalledWith({ limit: 500, level: 'WARN', module: null })
    expect(api.diagnosticsSetLevel).not.toHaveBeenCalled()
    expect(control('记录级别').value).toBe('INFO')
  })
  it('自动刷新支持暂停，页面卸载后不继续读取', async () => {
    await mount()
    await vi.advanceTimersByTimeAsync(2000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(2)
    control('自动刷新').props.onChange({ target: { checked: false } })
    await vi.advanceTimersByTimeAsync(4000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(2)
    control('自动刷新').props.onChange({ target: { checked: true } })
    await vi.advanceTimersByTimeAsync(2000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(3)
    memoryRenderer.render(null, root)
    await vi.advanceTimersByTimeAsync(4000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(3)
  })
  it('隐藏窗口不刷新，自动读取失败保留列表并暂停', async () => {
    await mount()
    vi.stubGlobal('document', { visibilityState: 'hidden' })
    await vi.advanceTimersByTimeAsync(2000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledOnce()
    vi.stubGlobal('document', { visibilityState: 'visible' })
    api.diagnosticsRecentLogs.mockRejectedValueOnce(new Error('读取失败'))
    await vi.advanceTimersByTimeAsync(2000)
    expect(text(root)).toContain('event=app.starting')
    expect(control('自动刷新').props.checked).toBe(false)
    await vi.advanceTimersByTimeAsync(4000)
    expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(2)
  })
})

it('后台刷新等待期间按钮、选择器、空列表提示保持稳定，且不重叠请求', async () => {
  api.diagnosticsRecentLogs.mockResolvedValueOnce({ ...recent, entries: [] })
  await mount()
  let complete!: (value: RecentLogs) => void
  api.diagnosticsRecentLogs.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  await vi.advanceTimersByTimeAsync(2000)
  expect(control('记录级别').props.disabled).toBe(false)
  expect(control('显示级别').props.disabled).toBe(false)
  expect(text(root)).toContain('当前条件下没有日志。')
  expect(text(root)).not.toContain('正在读取…')
  expect(text(root)).not.toContain('处理中…')
  expect(findNode(root, node => node.type === 'button' && text(node) === '刷新')!.props.disabled).toBe(false)
  await vi.advanceTimersByTimeAsync(4000)
  expect(api.diagnosticsRecentLogs).toHaveBeenCalledTimes(2)
  complete(recent); await flushComponent()
  expect(text(root)).toContain('event=app.starting')
})

it('用户切换记录级别等待后台读取结束，避免 IPC 冲突且不会丢失操作', async () => {
  await mount()
  let complete!: (value: RecentLogs) => void
  api.diagnosticsRecentLogs.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  await vi.advanceTimersByTimeAsync(2000)
  api.diagnosticsSetLevel.mockResolvedValueOnce({ ...status, level: 'DEBUG' })
  await choose('记录级别', 'DEBUG')
  expect(api.diagnosticsSetLevel).not.toHaveBeenCalled()
  complete(recent); await flushComponent()
  expect(api.diagnosticsSetLevel).toHaveBeenCalledWith('DEBUG')
  expect(control('记录级别').value).toBe('DEBUG')
})

it('手动刷新保留状态、选择器节点和已有日志', async () => {
  await mount()
  const selector = control('记录级别')
  let complete!: (value: DiagnosticsStatus) => void
  api.diagnosticsStatus.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  findNode(root, node => node.type === 'button' && text(node) === '刷新')!.props.onClick()
  await flushComponent()
  expect(control('记录级别')).toBe(selector)
  expect(text(root)).toContain('已就绪')
  expect(text(root)).toContain('event=app.starting')
  expect(text(root)).not.toContain('状态未读取')
  complete(status); await flushComponent()
})
