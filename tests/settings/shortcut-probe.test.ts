import { describe, expect, it, vi } from 'vitest'
import { shortcutProbeMessages, type ShortcutProbeReport } from '../../src/features/settings/shortcut-probe'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../../src/services/api'

function report(patch: Partial<ShortcutProbeReport> = {}): ShortcutProbeReport {
  return { enabled: true, editing: false, registrationError: null, registered: ['F1'], received: [],
    appElevated: false, target: { pid: 42, elevated: false, coversMonitor: false, keyboardLayout: '409' }, ...patch }
}

describe('特殊应用热键主动诊断', () => {
  it('只能发起固定测试，不接受窗口、进程或任意按键参数', async () => {
    const result = report()
    invoke.mockResolvedValueOnce(result)
    expect(await api.probeShortcuts()).toEqual(result)
    expect(invoke).toHaveBeenLastCalledWith('probe_shortcuts', undefined)
  })
  it.each([
    [{ enabled: false }, '已禁用'],
    [{ editing: true }, '正在录入'],
    [{ registrationError: 'F1 occupied' }, '系统注册失败'],
    [{ registered: [] }, '没有已注册'],
  ] as const)('配置不可测试时不能归因于目标软件：%j', (patch, text) => {
    const messages = shortcutProbeMessages(report(patch as Partial<ShortcutProbeReport>)).join(' ')
    expect(messages).toContain(text)
    expect(messages).not.toContain('在前台时未收到')
  })
  it('前台不是目标时的接收不能伪造目标内触发成功', () => {
    const messages = shortcutProbeMessages(report({ received: [{ shortcut: 'F1', foregroundPid: 99 }] })).join(' ')
    expect(messages).toContain('在前台时未收到')
    expect(messages).toContain('其他前台窗口')
    expect(messages).toContain('无法确定')
  })
  it('收到系统消息只证明投递成功，动作失败仍指向业务日志', () => {
    const messages = shortcutProbeMessages(report({ received: [{ shortcut: 'F1', foregroundPid: 42 }] })).join(' ')
    expect(messages).toContain('在前台时已收到：F1')
    expect(messages).toContain('查看日志')
    expect(messages).not.toContain('在前台时未收到')
  })
  it('未观察外部窗口时明确要求切换焦点重试', () => {
    expect(shortcutProbeMessages(report({ target: null })).join(' ')).toContain('未观察到外部前台窗口')
  })
  it('权限不可查询时保持未知，满屏不被误判为独占，输入法只给对比建议', () => {
    const messages = shortcutProbeMessages(report({ target: { pid: 42, elevated: null, coversMonitor: true, keyboardLayout: '804' } })).join(' ')
    expect(messages).toContain('无法比较权限')
    expect(messages).toContain('无法据此判断全屏独占')
    expect(messages).toContain('切换英文输入')
    expect(messages).not.toContain('提升状态相同')
  })
  it('权限差异提供可操作建议，不声称已确定失败原因', () => {
    const messages = shortcutProbeMessages(report({ appElevated: true })).join(' ')
    expect(messages).toContain('提升状态不同')
    expect(messages).toContain('将两者以普通权限运行后对比')
  })
})
