/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Rect } from '@/shared/geometry/selection'
import type { ScrollBackground, ScrollView } from '@/features/scroll/types'
import { invoke } from '@/services/ipc'

export const scrollApi = {
  startScroll: (id: number, rect: Rect, saveFrames: boolean, background: ScrollBackground = 'white') => invoke<ScrollView>('start_scroll', { id, rect, saveFrames, background }),
  controlScroll: (id: number, cancel = false, confirmedOnly = false) => invoke<void>('control_scroll', { id, cancel, confirmedOnly }),
  currentScroll: () => invoke<ScrollView | null>('current_scroll'),
}
