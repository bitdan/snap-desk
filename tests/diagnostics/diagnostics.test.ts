import { beforeEach, describe, expect, it, vi } from 'vitest'
import { searchLogs, type LogEntry } from '../../src/features/diagnostics/diagnostics'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/plugin-log', () => ({ error: vi.fn().mockResolvedValue(undefined), warn: vi.fn(), info: vi.fn(), debug: vi.fn() }))
import { api } from '../../src/services/api'

beforeEach(() => vi.clearAllMocks())

describe('本地诊断契约', () => {
  it('筛选仅发送受控查询，不允许传日志文件路径', async () => {
    const result = { entries: [], scannedBytes: 1024, omittedLines: 1, truncated: false }
    invoke.mockResolvedValueOnce(result)
    const query = { limit: 500, level: 'ERROR' as const, module: 'ocr' as const }
    expect(await api.diagnosticsRecentLogs(query)).toBe(result)
    expect(invoke).toHaveBeenLastCalledWith('diagnostics_recent_logs', { query })
    await api.diagnosticsStatus()
    expect(invoke).toHaveBeenLastCalledWith('diagnostics_status', undefined)
  })

  it('导出、清理不接收任意路径，保留取消语义与部分失败计数', async () => {
    invoke.mockResolvedValueOnce(false)
    expect(await api.diagnosticsExport()).toBe(false)
    expect(invoke).toHaveBeenLastCalledWith('diagnostics_export', undefined)
    const result = { cancelled: false, deleted: 3, failed: 1 }
    invoke.mockResolvedValueOnce(result)
    expect(await api.diagnosticsClearArchives()).toEqual(result)
    expect(invoke).toHaveBeenLastCalledWith('diagnostics_clear_archives', undefined)
  })

  it.each(['DEBUG', 'INFO', 'WARN', 'ERROR'] as const)('记录级别 %s 使用枚举契约，返回后端实际状态', async level => {
    const status = { level }
    invoke.mockResolvedValueOnce(status)
    expect(await api.diagnosticsSetLevel(level)).toBe(status)
    expect(invoke).toHaveBeenLastCalledWith('diagnostics_set_level', { level })
  })

  it('搜索只过滤已读内容，不进行 IPC；特殊字符按普通字符串处理', () => {
    const entries: LogEntry[] = [
      { timestamp: '2026-10-19T12:00:00+08:00', level: 'ERROR', module: 'ocr', message: 'event=ocr.failed code=OCR_BUSY' },
      { timestamp: '2026-10-19T12:00:01+08:00', level: 'INFO', module: 'app', message: 'event=app.starting' },
    ]
    expect(searchLogs(entries, ' oCr_bUsY ')).toEqual([entries[0]])
    expect(searchLogs(entries, '')).toBe(entries)
    expect(searchLogs(entries, '.*')).toEqual([])
    expect(invoke).not.toHaveBeenCalled()
  })
})
