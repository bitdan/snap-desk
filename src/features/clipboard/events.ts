/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */
import type { ClipboardStatus } from '@/features/clipboard/clipboard'
import { listen } from '@/services/ipc'

export const onClipboardChanged = (fn: () => void) => listen('clipboard-changed', fn)
export const onClipboardError = (fn: (error: string) => void) => listen<string>('clipboard-error', e => fn(e.payload))
export const onClipboardState = (fn: (state: ClipboardStatus) => void) => listen<ClipboardStatus>('clipboard-state', e => fn(e.payload))
export const onClipboardOpen = (fn: () => void) => listen('clipboard-open', fn)
/** 剪贴板窗口失去焦点；是否收起由窗口根据固定状态和子窗口决定。 */
export const onClipboardBlur = (fn: () => void) => listen('clipboard-blur', fn)
