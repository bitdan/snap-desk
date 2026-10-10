import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Rect } from './selection'
import type { Annotation } from './annotations'

export interface CaptureFrame { id: number; width: number; height: number; dataUrl: string }
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
export interface Settings { shortcut: string; delaySeconds: 0 | 3 | 5 }
export interface AppStatus { settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
export const desktop = isTauri()
export const api = {
  status: () => invoke<AppStatus>('app_status'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  cancelPending: () => invoke<void>('cancel_pending_capture'),
  start: () => invoke<void>('start_capture'),
  current: () => invoke<CaptureFrame | null>('current_capture'),
  ready: (id: number) => invoke<void>('show_capture', { id }),
  cancel: (id: number) => invoke<void>('cancel_capture', { id }),
  copy: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('copy_capture', { id, rect, annotations }),
  save: (id: number, rect: Rect, annotations: Annotation[]) => invoke<boolean>('save_capture', { id, rect, annotations }),
  hide: () => invoke<void>('hide_main'),
}
export const onFrame = (fn: (frame: CaptureFrame) => void) => listen<CaptureFrame>('capture-ready', e => fn(e.payload))
export const onError = (fn: (error: string) => void) => listen<string>('capture-error', e => fn(e.payload))
export const onResult = (fn: (result: string) => void) => listen<string>('capture-result', e => fn(e.payload))
export const onDelay = (fn: (remaining: number) => void) => listen<number>('capture-delay', e => fn(e.payload))
