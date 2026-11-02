/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'
import type { Settings } from '@/features/settings/settings'
import type { AppUpdateView } from '@/features/settings/types'
import { updatePreview, onPreviewUpdate } from './update-preview'

export const onAppUpdate = async (fn: (view: AppUpdateView) => void) => updatePreview ? onPreviewUpdate(fn) : listen<AppUpdateView>('app-update-changed', event => fn(event.payload))

export const onSettingsChanged = (fn: () => void) => listen('settings-changed', fn)

export const onSettingsImported = (fn: (settings: Settings) => void) => listen<Settings>('settings-imported', event => fn(event.payload))

// 同一主 WebView 内的打开意图；更新状态仍由原生端统一持有。
const updateWindowRequests = new Set<() => void>()
export function requestUpdateWindow() { updateWindowRequests.forEach(fn => fn()) }
export function onUpdateWindowRequested(fn: () => void) {
  updateWindowRequests.add(fn)
  return () => { updateWindowRequests.delete(fn) }
}
