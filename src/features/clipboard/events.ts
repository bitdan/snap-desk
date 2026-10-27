/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */
import type { ClipboardStatus } from '@/features/clipboard/clipboard'
import { listen } from '@/services/ipc'

export const onClipboardChanged = (fn: () => void) => listen('clipboard-changed', fn)
export const onClipboardError = (fn: (error: string) => void) => listen<string>('clipboard-error', e => fn(e.payload))
export const onClipboardState = (fn: (state: ClipboardStatus) => void) => listen<ClipboardStatus>('clipboard-state', e => fn(e.payload))
export const onClipboardOpen = (fn: () => void) => listen('clipboard-open', fn)
