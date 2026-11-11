/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { PinView, PinFrame, PinTransform, PinAction } from '@/features/pins/types'
import { invoke } from '@/services/ipc'

export const pinsApi = {
  finishHtmlRender: (id: number, png: string | null) => invoke<void>('finish_html_render', { id, png }),
  pin: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('pin_capture', { id, rect, annotations }),
  pinClipboard: () => invoke<void>('pin_clipboard'),
  pinFile: () => invoke<boolean>('pin_file'),
  currentPin: (knownRevision: number | null = null) => invoke<PinFrame>('current_pin', { knownRevision }),
  setPinAnnotations: (annotations: Annotation[]) => invoke<void>('set_pin_annotations', { annotations }),
  setPinView: (view: PinView) => invoke<PinView>('set_pin_view', { view }),
  dragPin: () => invoke<void>('drag_pin'),
  /** 物理像素、相对窗口左上角；null 恢复完整窗口。 */
  setPinRegion: (region: Rect | null) => invoke<void>('set_pin_region', { region }),
  dragPinContent: () => invoke<void>('drag_pin_content'),
  transformPin: (action: PinTransform) => invoke<PinFrame>('transform_pin', { action }),
  pinAction: (action: PinAction) => invoke<boolean>('pin_action', { action }),
}
