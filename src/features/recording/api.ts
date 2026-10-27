/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { RecordingAnnotationView } from '@/features/recording/recording-annotations'
import type { RecordingEditorView, RecordingTrimRange, RecordingTrimExport } from '@/features/recording/recording-editor'
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { RecorderCapabilities, RecorderConfig, RecordingAction, RecordingDirectory, RecordingView, RecoverableRecording } from '@/features/recording/recording'
import type { RecordingResult, RecordingResultAction } from '@/features/recording/recording'
import { invoke } from '@/services/ipc'

export const recordingApi = {
  prepareRecordingEditor: (id: number) => invoke<RecordingEditorView>('prepare_recording_editor', { id }),
  closeRecordingEditor: (token: number) => invoke<void>('close_recording_editor', { token }),
  trimRecording: (token: number, range: RecordingTrimRange) => invoke<RecordingTrimExport | null>('trim_recording', { token, range }),
  cancelRecordingTrim: (token: number) => invoke<void>('cancel_recording_trim', { token }),
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
  setRecordingAnnotationMode: (id: number, editing: boolean, toolbar?: Rect) => invoke<void>('set_recording_annotation_mode', { id, editing, ...(toolbar ? { toolbar } : {}) }),
  updateRecordingAnnotations: (id: number, shapes: Annotation[]) => invoke<RecordingAnnotationView>('update_recording_annotations', { id, shapes }),
  controlRecording: (id: number, action: RecordingAction) => invoke<void>('control_recording', { id, action }),
}
