/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
export interface CaptureReplay { index: number; total: number; rect: Rect; annotations: Annotation[] }
export interface CaptureTarget { rect: Rect; kind: 'window' | 'element'; windowIndex: number; title?: string; appName?: string; recordingOnly?: boolean; icon?: string | null }
export interface CaptureDisplay { x: number; y: number; width: number; height: number; primary: boolean; scale: number }
export interface CaptureFrame { originX: number; originY: number; displays: CaptureDisplay[]; targets: CaptureTarget[]; layoutKey: string; id: number; width: number; height: number; previewRevision: number; recording: boolean; snapEnabled?: boolean; replay?: CaptureReplay | null }
export type ColorFormat = 'hex' | 'rgb'
/** 前端区间使用 performance 时钟；首帧总耗时由原生入口到回执计算。 */
export interface CapturePaint { loadMs: number; decodeMs: number; prePaintMs: number; postPaintMs: number; paintSource: 'raf' | 'timeout' }
