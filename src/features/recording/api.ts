/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { RecordingAnnotationView, RecordingAnnotationTools, RecordingAnnotationEditorAction, RecordingAnnotationEditorStatus } from '@/features/recording/recording-annotations'
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { RecorderCapabilities, RecorderConfig, RecordingAction, RecordingDirectory, RecordingView, RecoverableRecording } from '@/features/recording/recording'
import type { RecordingResult, RecordingResultAction } from '@/features/recording/recording'
import { invoke } from '@/services/ipc'

export const recordingApi = {
  latestRecordingResult: () => invoke<RecordingResult | null>('latest_recording_result'),
  recordingResultAction: (id: number, action: RecordingResultAction) => invoke<void>('recording_result_action', { id, action }),
  selectRecordingArea: () => invoke<void>('select_recording_area'),
  recoverableRecordings: () => invoke<RecoverableRecording[]>('recoverable_recordings'),
  retryRecordingExport: (id: string) => invoke<string>('retry_recording_export', { id }),
  discardRecordingRecovery: (id: string) => invoke<void>('discard_recording_recovery', { id }),
  startRecording: (id: number, rect: Rect, config?: RecorderConfig, target?: number, annotations?: Annotation[]) => invoke<void>('start_recording', { id, rect, config, ...(target === undefined ? {} : { target }), ...(annotations?.length ? { annotations } : {}) }),
  recordingCapabilities: () => invoke<RecorderCapabilities>('recording_capabilities'),
  currentRecordingDirectory: () => invoke<RecordingDirectory>('current_recording_directory'),
  chooseRecordingDirectory: (reset = false) => invoke<RecordingDirectory | null>('choose_recording_directory', { reset }),
  currentRecording: () => invoke<RecordingView | null>('current_recording'),
  recordingAnnotations: (knownRevision?: number) => invoke<RecordingAnnotationView | null>('current_recording_annotations', { knownRevision: knownRevision ?? null }),
  setRecordingAnnotationMode: (id: number, editing: boolean) => invoke<void>('set_recording_annotation_mode', { id, editing }),
  setRecordingAnnotationRegions: (id: number, regions: Rect[]) => invoke<void>('set_recording_annotation_regions', { id, regions }),
  recordingAnnotationEditorAction: (id: number, action: RecordingAnnotationEditorAction) => invoke<void>('recording_annotation_editor_action', { id, action }),
  finishRecordingAnnotationEditorAction: (id: number, requestId: number, error: string | null) => invoke<void>('finish_recording_annotation_editor_action', { id, requestId, error }),
  publishRecordingAnnotationEditor: (id: number, editor: RecordingAnnotationEditorStatus, tools: RecordingAnnotationTools) => invoke<void>('publish_recording_annotation_editor', { id, editor, tools }),
  setRecordingAnnotationTools: (id: number, tools: RecordingAnnotationTools) => invoke<void>('set_recording_annotation_tools', { id, tools }),
  updateRecordingAnnotations: (id: number, shapes: Annotation[], knownRevision?: number) => invoke<RecordingAnnotationView>('update_recording_annotations', { id, shapes, knownRevision: knownRevision ?? null }),
  controlRecording: (id: number, action: RecordingAction) => invoke<void>('control_recording', { id, action }),
}
