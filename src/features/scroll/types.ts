/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Rect } from '@/shared/geometry/selection'
export type ScrollBackground = 'white' | 'transparent'
export interface ScrollView {
  id: number; revision: number; collecting: boolean; archivePath: string | null;
  width: number; height: number; frames: number; message: string; preview: string; frameHeight: number
  canvasBounds: Rect; viewportRect: Rect; coveragePercent: number; hasHoles: boolean
  direction: 'none' | 'up' | 'down' | 'left' | 'right'
  status: 'sampling' | 'no_progress' | 'waiting' | 'collecting' | 'recovering' | 'finishing' | 'failed'
}
