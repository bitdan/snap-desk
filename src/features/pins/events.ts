import type { PinAppearance, PinView } from './types'
/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

export const onPinsChanged = (fn: () => void) => listen('pins-changed', fn)
export const onPinVisibility = (fn: (visible: boolean) => void) => listen<boolean>('pin-visibility', event => fn(event.payload), getCurrentWebviewWindow().label)

export const onPinAppearance = (fn: (value: PinAppearance) => void) => listen<PinAppearance>('pin-appearance-changed', event => fn(event.payload))

export const onPinView = (fn: (view: PinView) => void) => listen<PinView>('pin-view-changed', event => fn(event.payload), getCurrentWebviewWindow().label)
export const onHtmlPinRender = (fn: (request: { id: number; html: string }) => void) => listen<{ id: number; html: string }>('pin-render-html', event => fn(event.payload), 'main')
