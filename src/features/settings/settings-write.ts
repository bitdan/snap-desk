import { api } from '@/services/api'
import type { Settings } from './settings'

let writes: Promise<unknown> = Promise.resolve()
/** 自动保存与导入导出共用串行队列，避免旧写入跨过整份配置替换。 */
export function queueSettingsOperation<T>(operation: () => Promise<T>): Promise<T> {
  const next = writes.catch(() => {}).then(operation)
  writes = next
  return next
}
/** 读取最新生效配置后，仅合并当前模块负责的字段。 */
export function updateSettings(update: (current: Settings) => Settings): Promise<Settings> {
  return queueSettingsOperation(async () => api.saveSettings(update((await api.status()).settings)))
}
