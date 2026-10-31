import { isTauri } from '@tauri-apps/api/core'
import { debug, error, info, warn } from '@tauri-apps/plugin-log'
import { ipcCommands, type IpcCommand } from '@/services/ipc-commands'

type Level = 'debug' | 'info' | 'warn' | 'error'
type ErrorMeta = { kind?: string; line?: number; column?: number }
type EventMeta = {
  'ipc.failed': { command: IpcCommand; session?: unknown }
  'frontend.uncaught_error': ErrorMeta | undefined
  'frontend.unhandled_rejection': ErrorMeta | undefined
  'frontend.appearance_listener_failed': ErrorMeta | undefined
  'frontend.mount_failed': ErrorMeta | undefined
  'vue.component_error': ErrorMeta | undefined
}
const events = new Set(['ipc.failed', 'frontend.uncaught_error', 'frontend.unhandled_rejection', 'frontend.mount_failed', 'frontend.appearance_listener_failed', 'vue.component_error'])
const commands = new Set<string>(ipcCommands)
const writers = { debug, error, info, warn }
const errorKinds = new Set(['Error', 'TypeError', 'ReferenceError', 'SyntaxError', 'RangeError', 'URIError', 'EvalError', 'AggregateError', 'DOMException', 'non_error', 'unknown'])
let windowKind = 'unknown'
/** 只接受应用固定窗口类别，不记录带用户数据的窗口标签或 URL。 */
export function setLogWindowKind(kind: string): void {
  windowKind = ['main', 'capture', 'pin', 'selection-edge', 'recording-controls', 'scroll-preview', 'scroll-controls'].includes(kind) ? kind : 'unknown'
}
/** 只取固定异常类别；异常正文、堆栈和任意自定义 name 均不写盘。 */
function errorKind(value: unknown): string {
  try {
    if (value instanceof Error || (typeof DOMException !== 'undefined' && value instanceof DOMException)) {
      return errorKinds.has(value.name) ? value.name : 'unknown'
    }
    return value == null ? 'unknown' : 'non_error'
  } catch { return 'unknown' }
}
// 只关联后端已有的数值会话 ID；文本 requestId、文件 ID 和任意参数不写盘。
const sessionPrefixes: Partial<Record<IpcCommand, string>> = {
  copy_capture: 'cap', save_capture: 'cap', pin_capture: 'cap',
  recognize_capture_text: 'cap', recognize_capture_v2: 'cap',
  start_scroll: 'scroll', control_scroll: 'scroll',
  start_recording: 'rec', control_recording: 'rec',
}

/** 唯一日志入口：事件与字段均为白名单，不序列化异常、参数或额外属性。 */
export function appLog<E extends keyof EventMeta>(level: Level, event: E, ...args: undefined extends EventMeta[E] ? [meta?: EventMeta[E]] : [EventMeta[E]]): void {
  try {
    if (!isTauri() || !events.has(event)) return
    let line = `event=${event} module=frontend`
    if (event === 'ipc.failed') {
      const meta = args[0] as EventMeta['ipc.failed'] | undefined
      const command = meta?.command
      if (!command || !commands.has(command)) return
      line += ` command=${command} code=IPC_FAILED`
      const session = meta?.session
      const prefix = sessionPrefixes[command]
      if (prefix && typeof session === 'number' && Number.isSafeInteger(session) && session >= 0) {
        line += ` op=${prefix}-${session}`
      }
    }
    if (event !== 'ipc.failed' && args[0]) {
      const meta = args[0] as ErrorMeta | undefined
      line += ` window=${windowKind} error_kind=${meta?.kind && errorKinds.has(meta.kind) ? meta.kind : 'unknown'}`
      for (const key of ['line', 'column'] as const) {
        const value = meta?.[key]
        if (typeof value === 'number' && Number.isSafeInteger(value) && value > 0) line += ` ${key}=${value}`
      }
    }
    // 同步异常与异步拒绝都吞掉；日志不可递归记录自己的失败。
    void writers[level](line).catch(() => {})
  } catch { /* 不影响原有业务和错误展示。 */ }
}

/** 启动和 Vue 错误复用相同的类别白名单，展示文案仍由调用方保留。 */
export function logFrontendFailure(event: Exclude<keyof EventMeta, 'ipc.failed'>, error: unknown): void {
  appLog('error', event, { kind: errorKind(error) })
}

/** 只识别后端现有的精确取消契约，不能用包含“取消”的模糊匹配隐藏真实故障。 */
export function logIpcFailure(command: IpcCommand, message: string, session?: unknown): void {
  if ((command === 'translate_text' || command === 'translate_texts') && message === '翻译已取消。') return
  appLog('error', 'ipc.failed', { command, session })
}

const installed = new WeakSet<EventTarget>()
/** 多窗口各自安装一次；只提取异常类别与数字位置，不序列化异常对象。 */
export function installGlobalErrorLogging(target: EventTarget): void {
  if (installed.has(target)) return
  installed.add(target)
  target.addEventListener('error', event => {
    const error = event as ErrorEvent
    appLog('error', 'frontend.uncaught_error', { kind: errorKind(error.error), line: error.lineno, column: error.colno })
  })
  target.addEventListener('unhandledrejection', event => appLog('error', 'frontend.unhandled_rejection', { kind: errorKind((event as PromiseRejectionEvent).reason) }))
}
