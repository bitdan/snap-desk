/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'
import type { Settings } from '@/features/settings/settings'
import type { AppUpdateView } from '@/features/settings/types'

export const onAppUpdate = (fn: (view: AppUpdateView) => void) => listen<AppUpdateView>('app-update-changed', event => fn(event.payload))

export const onSettingsChanged = (fn: () => void) => listen('settings-changed', fn)

export const onSettingsImported = (fn: (settings: Settings) => void) => listen<Settings>('settings-imported', event => fn(event.payload))
