/** 与原生主动测试的报告契约对齐；未知权限与未观察到目标窗口均保持显式状态。 */
export interface ShortcutProbeReport {
  enabled: boolean
  editing: boolean
  registrationError: string | null
  registered: string[]
  received: { shortcut: string; foregroundPid: number }[]
  appElevated: boolean | null
  target: { pid: number; elevated: boolean | null; coversMonitor: boolean | null; keyboardLayout: string } | null
}

/** 观测事实与建议分开：满屏几何信息不能证明独占，权限差异也不能证明拦截。 */
export function shortcutProbeMessages(report: ShortcutProbeReport): string[] {
  const messages: string[] = []
  if (!report.enabled) messages.push('全局快捷键已禁用，请先启用后重新测试。')
  if (report.editing) messages.push('正在录入快捷键，系统注册已临时释放；结束录入后重试。')
  if (report.registrationError) messages.push(`系统注册失败：${report.registrationError}。关闭占用此组合的软件或更换组合，再保存快捷键。`)
  if (!report.registered.length) messages.push('当前没有已注册的快捷键；请设置至少一个动作并保存。')
  if (messages.length) return messages
  messages.push(`已注册：${report.registered.join('、')}。`)
  const target = report.target
  if (!target) {
    messages.push('未观察到外部前台窗口；重新测试并在八秒内切换到目标软件按键。')
    return messages
  }
  const received = report.received.filter(item => item.foregroundPid === target.pid)
  messages.push(received.length
    ? `目标进程 PID ${target.pid} 在前台时已收到：${[...new Set(received.map(item => item.shortcut))].join('、')}。若实际动作仍失败，请查看日志中的截图或录屏错误。`
    : `目标进程 PID ${target.pid} 在前台时未收到快捷键；本次观测无法确定它是否拦截了按键。请确认使用已保存组合并重新测试。`)
  if (report.received.some(item => item.foregroundPid !== target.pid)) messages.push('其他前台窗口收到过热键；这不代表目标软件内也能触发。')
  if (target.elevated === null || report.appElevated === null) messages.push('进程权限读取失败，无法比较权限级别；受保护进程可能禁止查询。')
  else if (target.elevated !== report.appElevated) messages.push('目标软件与 Snap Desk 的管理员提升状态不同；可先将两者以普通权限运行后对比，勿为此关闭系统安全保护。')
  else messages.push('目标软件与 Snap Desk 的管理员提升状态相同；此检查不代表所有进程安全属性一致。')
  if (target.coversMonitor) messages.push('目标窗口覆盖显示器，但无法据此判断全屏独占；尝试窗口化或无边框模式后对比。')
  if (target.coversMonitor === null) messages.push('无法读取目标窗口的物理边界，满屏状态未知。')
  messages.push(`目标键盘布局：${target.keyboardLayout === '0' ? '未知' : target.keyboardLayout}。若组合被输入法使用，切换英文输入或改用 F 键组合后对比。`)
  return messages
}
