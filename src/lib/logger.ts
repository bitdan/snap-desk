import { isTauri } from '@tauri-apps/api/core'
import { debug, error, info, warn } from '@tauri-apps/plugin-log'
import { ipcCommands, type IpcCommand } from './ipc-commands'

type Level = 'debug' | 'info' | 'warn' | 'error'
type EventMeta = {
  'ipc.failed': { command: IpcCommand; session?: unknown }
  'frontend.uncaught_error': undefined
  'frontend.unhandled_rejection': undefined
  'frontend.mount_failed': undefined
  'vue.component_error': undefined
}
const events = new Set(['ipc.failed', 'frontend.uncaught_error', 'frontend.unhandled_rejection', 'frontend.mount_failed', 'vue.component_error'])
const commands = new Set<string>(ipcCommands)
const writers = { debug, error, info, warn }
// 只关联后端已有的数值会话 ID；文本 requestId、文件 ID 和任意参数不写盘。
const sessionPrefixes: Partial<Record<IpcCommand, string>> = {
  copy_capture: 'cap', save_capture: 'cap', pin_capture: 'cap',
  recognize_capture_text: 'cap', recognize_capture_v2: 'cap',
  start_scroll: 'scroll', control_scroll: 'scroll',
  start_recording: 'rec', control_recording: 'rec',
}

/** 唯一日志入口：事件与字段均为白名单，不序列化异常、参数或额外属性。 */
export function appLog<E extends keyof EventMeta>(level: Level, event: E, ...args: EventMeta[E] extends undefined ? [] : [EventMeta[E]]): void {
  try {
    if (!isTauri() || !events.has(event)) return
    let line = `event=${event} module=frontend`
    if (event === 'ipc.failed') {
      const command = args[0]?.command
      if (!command || !commands.has(command)) return
      line += ` command=${command} code=IPC_FAILED`
      const session = args[0]?.session
      const prefix = sessionPrefixes[command]
      if (prefix && typeof session === 'number' && Number.isSafeInteger(session) && session >= 0) {
        line += ` op=${prefix}-${session}`
      }
    }
    // 同步异常与异步拒绝都吞掉；日志不可递归记录自己的失败。
    void writers[level](line).catch(() => {})
  } catch { /* 不影响原有业务和错误展示。 */ }
}

/** 只识别后端现有的精确取消契约，不能用包含“取消”的模糊匹配隐藏真实故障。 */
export function logIpcFailure(command: IpcCommand, message: string, session?: unknown): void {
  if (command === 'translate_text' && message === '翻译已取消。') return
  appLog('error', 'ipc.failed', { command, session })
}

const installed = new WeakSet<EventTarget>()
/** 多窗口各自安装一次；监听器不读取 ErrorEvent 的 message、堆栈或 Promise 原因。 */
export function installGlobalErrorLogging(target: EventTarget): void {
  if (installed.has(target)) return
  installed.add(target)
  target.addEventListener('error', () => appLog('error', 'frontend.uncaught_error'))
  target.addEventListener('unhandledrejection', () => appLog('error', 'frontend.unhandled_rejection'))
}
