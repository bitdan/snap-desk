import { afterEach, describe, expect, it, vi } from 'vitest'
import { recoveryBytes, recoveryItemStatus, recoverySummary, recoveryTime, type RecoveryItem, type RecoveryStatus } from '../src/lib/recovery-center'
import { appearance } from '../src/lib/appearance'

afterEach(() => { appearance.value.language = 'zh-CN' })

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../src/lib/api'

function item(patch: Partial<RecoveryItem> = {}): RecoveryItem {
  return { id: 'snapdesk-pending-test123', kind: 'export', title: '截图导出', path: 'F:\\screens\\snapdesk-pending-test123', bytes: 1024, modifiedAt: null, ready: true, sourceExists: true, stage: 'encoded', ...patch }
}

describe('启动恢复中心', () => {
  it('缺失体积不能假报零，汇总说明未知项目数量', () => {
    expect(recoveryBytes(null)).toBe('体积未知')
    expect(recoveryBytes(0)).toBe('0 B')
    expect(recoveryBytes(1_048_576)).toBe('1.0 MiB')
    expect(recoverySummary([item(), item({ bytes: null })])).toBe('2 项 · 1.0 KiB，1 项体积未知')
    expect(recoverySummary([])).toBe('0 项 · 0 B')
  })
  it('未封装录屏与未编码图片不能显示可恢复', () => {
    expect(recoveryItemStatus(item({ kind: 'recording', ready: false }))).toContain('未确认完成封装')
    expect(recoveryItemStatus(item({ ready: false }))).toContain('编码未完成')
    expect(recoveryItemStatus(item())).toContain('可恢复导出')
  })
  it('缺失源文件和未知目录内容保持明确，批次可人工检查', () => {
    expect(recoveryItemStatus(item({ sourceExists: false, ready: false }))).toContain('源文件缺失')
    expect(recoveryItemStatus(item({ kind: 'temporary', ready: false }))).toContain('打开目录检查')
    expect(recoveryItemStatus(item({ stage: 'unsafe', ready: false }))).toContain('自动清理会拒绝')
    expect(recoveryItemStatus(item({ kind: 'recording', stage: 'cleanup-pending' }))).toContain('导出后残留')
  })
  it.each(['zh-CN', 'en'] as const)('缺失时间保持未知，正常时间跟随应用语言 %s', language => {
    appearance.value.language = language
    expect(recoveryTime(null)).toBe(language === 'en' ? 'Unknown time' : '时间未知')
    // 应用语言可以与操作系统不同，预期值不能依赖 CI 机器的默认 locale。
    const locale = language === 'en' ? 'en-US' : 'zh-CN'
    expect(recoveryTime(1792727969001)).toBe(new Date(1792727969001).toLocaleString(locale))
  })
  it('启动查询返回上次运行信息与坏记录计数，不接收任意扫描目录', async () => {
    const status: RecoveryStatus = { previousStartedAt: 1792727969001, previousPid: 42, warning: null, items: [item()], unreadableRecords: 2 }
    invoke.mockResolvedValueOnce(status)
    expect(await api.recoveryStatus()).toEqual(status)
    expect(invoke).toHaveBeenLastCalledWith('recovery_status', undefined)
  })
  it('恢复、打开和删除只发送后台 ID 与受限动作，取消另存保持 null', async () => {
    for (const action of ['recover', 'openDirectory', 'discard'] as const) {
      invoke.mockResolvedValueOnce(null)
      expect(await api.recoveryArtifactAction('snapdesk-pending-test123', action)).toBeNull()
      expect(invoke).toHaveBeenLastCalledWith('recovery_artifact_action', { id: 'snapdesk-pending-test123', action })
    }
    await api.openAppDirectory('data')
    expect(invoke).toHaveBeenLastCalledWith('open_app_directory', { kind: 'data' })
  })
})
