import { describe, expect, it, vi } from 'vitest'
import { defaultSettings, validateShortcuts } from '../src/lib/settings'
import { defaultClipboardSettings, reorderSelection, toggleSelection } from '../src/lib/clipboard'
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))
import { api, onClipboardChanged, onClipboardError, onClipboardState, onClipboardOpen } from '../src/lib/api'

describe('历史配置及选择顺序', () => {
  it('默认关闭，不共享可变配置，历史键参加冲突检测', () => {
    const settings = defaultSettings()
    expect(settings.clipboard.enabled).toBe(false)
    settings.clipboard.enabled = true
    expect(defaultSettings().clipboard.enabled).toBe(false)
    expect(settings.clipboard.saveCopiedFiles).toBe(true)
    expect(settings.shortcuts.history).toBe('F4')
    expect(validateShortcuts({ ...settings.shortcuts, capture: 'F4' })).toContain('冲突')
  })
  it('按点击顺序多选，重排不修改源数组且拒绝无效拖动', () => {
    const ids = toggleSelection(toggleSelection([], 9), 2)
    expect(ids).toEqual([9, 2])
    expect(reorderSelection(ids, 0, 1)).toEqual([2, 9])
    expect(ids).toEqual([9, 2])
    expect(reorderSelection(ids, NaN, 0)).toEqual(ids)
    expect(toggleSelection(ids, 9)).toEqual([2])
    const full = Array.from({ length: 100 }, (_, i) => i + 1)
    expect(toggleSelection(full, 101)).toHaveLength(100)
  })
})
describe('历史 IPC 契约', () => {
  it('查询、操作与导出只提交类型化参数和 ID', async () => {
    await api.clipboardList({ keyword: '中文%', kind: 'text', limit: 40, cursor: '123:9' })
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_list', { query: { keyword: '中文%', kind: 'text', limit: 40, cursor: '123:9' } })
    await api.clipboardList({ keyword: 'report.pdf', kind: 'file', limit: 40 })
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_list', { query: { keyword: 'report.pdf', kind: 'file', limit: 40 } })
    await api.clipboardClear(false)
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_clear', { includeFavorites: false })
    await api.clipboardFavorite(9, true)
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_set_favorite', { id: 9, favorite: true })
    await api.clipboardConfigure(defaultClipboardSettings())
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_configure', { settings: defaultClipboardSettings() })
    const options = { ids: [9, 2], mode: 'pdf' as const, pageSize: 'A4' as const, gapPx: 16, marginMm: 12, background: 'white' as const }
    await api.clipboardExport(options)
    expect(mocks.invoke).toHaveBeenLastCalledWith('clipboard_export', { options })
  })
  it('订阅更新、错误、状态及唤起事件', async () => {
    const callback = vi.fn()
    await onClipboardChanged(callback); expect(mocks.listen).toHaveBeenLastCalledWith('clipboard-changed', callback)
    await onClipboardOpen(callback); expect(mocks.listen).toHaveBeenLastCalledWith('clipboard-open', callback)
    await onClipboardError(callback)
    mocks.listen.mock.calls.at(-1)![1]({ payload: '磁盘不可用' }); expect(callback).toHaveBeenLastCalledWith('磁盘不可用')
    await onClipboardState(callback)
    mocks.listen.mock.calls.at(-1)![1]({ payload: { enabled: false } }); expect(callback).toHaveBeenLastCalledWith({ enabled: false })
  })
})
