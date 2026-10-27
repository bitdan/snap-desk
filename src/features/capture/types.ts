/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Rect } from '@/shared/geometry/selection'
export interface CaptureTarget { rect: Rect; kind: 'window' | 'element'; windowIndex: number; title?: string; appName?: string; recordingOnly?: boolean; icon?: string | null }
export interface CaptureDisplay { x: number; y: number; width: number; height: number; primary: boolean; scale: number }
export interface CaptureFrame { originX: number; originY: number; displays: CaptureDisplay[]; targets: CaptureTarget[]; layoutKey: string; id: number; width: number; height: number; previewRevision: number; recording: boolean; snapEnabled?: boolean }
export type ColorFormat = 'hex' | 'rgb'
