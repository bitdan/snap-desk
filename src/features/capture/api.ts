/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Point, Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { CaptureFrame, ColorFormat } from '@/features/capture/types'
import { invoke } from '@/services/ipc'

export const captureApi = {
  cancelPending: () => invoke<void>('cancel_pending_capture'),
  start: () => invoke<void>('start_capture'),
  nativeWindow: (id: number, target: number) => invoke<CaptureFrame>('capture_native_window', { id, target }),
  current: () => invoke<CaptureFrame | null>('current_capture'),
  ready: (id: number) => invoke<void>('show_capture', { id }),
  cancel: (id: number) => invoke<void>('cancel_capture', { id }),
  copy: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('copy_capture', { id, rect, annotations }),
  copyColor: (id: number, point: Point, format: ColorFormat) => invoke<void>('copy_capture_color', { id, point, format }),
  save: (id: number, rect: Rect, annotations: Annotation[]) => invoke<boolean>('save_capture', { id, rect, annotations }),
}
