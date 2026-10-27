/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */
// 关闭事件按目标窗口订阅，防止滚动辅助窗口误消费截图接管选区的通知。
import type { CaptureFrame } from '@/features/capture/types'
import { listen } from '@/services/ipc'

export const onFrame = (fn: (frame: CaptureFrame) => void) => listen<CaptureFrame>('capture-ready', e => fn(e.payload))
export const onCaptureClosed = (fn: (id: number) => void, target: 'capture' | 'scroll-preview' | 'scroll-controls') =>
  listen<number>('capture-closed', e => fn(e.payload), target)
export const onCapturePin = (fn: (id: number) => void) => listen<number>('capture-pin-requested', e => fn(e.payload))
export const onError = (fn: (error: string) => void) => listen<string>('capture-error', e => fn(e.payload))
export const onResult = (fn: (result: string) => void) => listen<string>('capture-result', e => fn(e.payload))
export const onDelay = (fn: (remaining: number) => void) => listen<number>('capture-delay', e => fn(e.payload))
