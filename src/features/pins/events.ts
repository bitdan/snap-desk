import type { PinAppearance } from './types'
/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

export const onPinsChanged = (fn: () => void) => listen('pins-changed', fn)
export const onPinVisibility = (fn: (visible: boolean) => void) => listen<boolean>('pin-visibility', event => fn(event.payload), getCurrentWebviewWindow().label)

export const onPinAppearance = (fn: (value: PinAppearance) => void) => listen<PinAppearance>('pin-appearance-changed', event => fn(event.payload))
