import type { Annotation } from '@/features/annotations/annotations'
import { shapeBounds } from '@/features/annotations/annotations'
import type { AnnotationTool, AnnotationEditor } from '@/features/annotations/use-annotations'
import { prepareRaster } from '@/features/annotations/annotation-raster'
/** 物理像素坐标；浮层按显示器尺寸换算比例，兼容不同 DPI。 */
export interface RecordingAnnotationView {
  id: number; revision: number; width: number; height: number
  viewportWidth: number; viewportHeight: number; x: number; y: number
  editing: boolean; shapes?: Annotation[]
  mediaTimeMs: number; tools: RecordingAnnotationTools; editor: RecordingAnnotationEditorStatus; tracks?: TimedAnnotation[]
}

/** 实时滤镜只提交范围，原生线程针对每帧处理；文字仍复用截图的字形渲染。 */
export const prepareRecordingRaster: typeof prepareRaster = async (shape, url, crop, preserve) => {
  if (!['mosaic', 'blur', 'magnify'].includes(shape.kind)) return prepareRaster(shape, url, crop, preserve)
  const box = shapeBounds(shape)
  return { ...shape, start: { x: box.x, y: box.y }, end: { x: box.x + box.width, y: box.y + box.height },
    raster: undefined, opacity: shape.kind === 'magnify' ? shape.opacity ?? 1 : 1 }
}

/** 时间全部来自 Rust 媒体 PTS，暂停期间不推进。 */
export interface TimedAnnotation { id: string; shape: Annotation; startMs: number; endMs: number | null; fadeOutMs: number; order: number }
export interface RecordingAnnotationTools {
  tool: AnnotationTool; color: string; width: number; opacity: number; fontSize: number; fontFamily: 'sans' | 'serif' | 'mono'; magnification: number; lifetimeMs: number; fadeOutMs: number
}
export function annotationAlpha(track: TimedAnnotation, ms: number): number {
  if (ms < track.startMs || (track.endMs !== null && ms >= track.endMs)) return 0
  return track.endMs !== null && track.fadeOutMs > 0 ? Math.min(1, (track.endMs - ms) / Math.max(1, Math.min(track.fadeOutMs, track.endMs - track.startMs))) : 1
}
export interface RecordingAnnotationEditorStatus { ready: boolean; selected: number; pending: boolean; interacting: boolean; canUndo: boolean; canRedo: boolean; text: string; error: string }
export type RecordingAnnotationEditorAction = { kind: 'tool'; tool: AnnotationTool } | { kind: 'style'; style: Parameters<AnnotationEditor['style']>[0] } | { kind: 'undo' | 'redo' | 'remove' | 'clear' }
export interface RecordingAnnotationEditorRequest { id: number; requestId: number; action: RecordingAnnotationEditorAction }
