import { api } from '@/services/api'
import type { Settings } from './settings'

let writes: Promise<unknown> = Promise.resolve()
/** 自动保存共用写队列，每次读取最新配置，仅替换本次修改的领域字段。 */
export function updateSettings(update: (current: Settings) => Settings): Promise<Settings> {
  const next = writes.catch(() => {}).then(async () => api.saveSettings(update((await api.status()).settings)))
  writes = next
  return next
}
