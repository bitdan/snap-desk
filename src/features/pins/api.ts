/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { PinView, PinFrame, PinSummary, PinTransform, PinAction, PinManageAction } from '@/features/pins/types'
import { invoke } from '@/services/ipc'

export const pinsApi = {
  pin: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('pin_capture', { id, rect, annotations }),
  pinClipboard: () => invoke<void>('pin_clipboard'),
  pinFile: () => invoke<boolean>('pin_file'),
  currentPin: () => invoke<PinFrame>('current_pin'),
  setPinAnnotations: (annotations: Annotation[]) => invoke<void>('set_pin_annotations', { annotations }),
  listPins: () => invoke<PinSummary[]>('list_pins'),
  managePins: (action: PinManageAction, label: string | null = null, group: string | null = null) => invoke<void>('manage_pins', { action, label, group }),
  setPinGroup: (labels: string[], group: string) => invoke<void>('set_pin_group', { labels, group }),
  setPinView: (view: PinView) => invoke<PinView>('set_pin_view', { view }),
  dragPin: () => invoke<void>('drag_pin'),
  transformPin: (action: PinTransform) => invoke<PinFrame>('transform_pin', { action }),
  pinAction: (action: PinAction) => invoke<boolean>('pin_action', { action }),
}
