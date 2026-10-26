import type { RecordingAnnotationView } from './recording-annotations'
import type { RecordingEditorView, RecordingTrimRange, RecordingTrimExport } from './recording-editor'
import type { DiagnosticsStatus, LogQuery, RecentLogs, ClearArchivesResult } from './diagnostics'
import type { ClipboardSettings, ClipboardQuery, ClipboardPage, ClipboardStatus, ClipboardExportOptions, ClipboardExportResult } from './clipboard'
import { invoke as nativeInvoke, isTauri, type InvokeArgs } from '@tauri-apps/api/core'
import { listen as nativeListen, type EventCallback } from '@tauri-apps/api/event'
import { errorMessage } from './errors'
import { logIpcFailure } from './logger'
import type { IpcCommand } from './ipc-commands'
import type { Point, Rect } from './selection'
import type { Annotation } from './annotations'
import type { AiSettings, OcrResult, TranslationResult, LocalTranslationStatus } from './ai'
import type { Settings } from './settings'
import type { ShortcutProbeReport } from './shortcut-probe'
import type { RecoveryStatus, RecoveryArtifactAction } from './recovery-center'
import type { RecorderCapabilities, RecorderConfig, RecordingAction, RecordingDirectory, RecordingView, RecoverableRecording } from './recording'
import type { RecordingResult, RecordingResultAction } from './recording'
export type { Settings } from './settings'

export interface QrResult { id: number; items: { text: string; url: string | null }[] }
export interface ResourceStatus { directory: string; ocrReady: boolean; modelsReady: boolean; runtimeReady: boolean; runtimeDownloadMib: number; fontReady: boolean }
export interface ResourceProgress { name: string; received: number; total: number }
export interface CaptureTarget { rect: Rect; kind: 'window' | 'element'; windowIndex: number }
export interface CaptureDisplay { x: number; y: number; width: number; height: number; primary: boolean; scale: number }
export interface CaptureFrame { originX: number; originY: number; displays: CaptureDisplay[]; targets: CaptureTarget[]; layoutKey: string; id: number; width: number; height: number; previewRevision: number; recording: boolean }
export interface PinView { zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
export interface PinFrame { label: string; width: number; height: number; dataUrl: string; view: PinView; transparencySupported: boolean; annotations: Annotation[] }
export interface PinSummary { label: string; width: number; height: number; visible: boolean; group: string }
export type ColorFormat = 'hex' | 'rgb'
export type ScrollMode = 'auto' | 'vertical' | 'horizontal'
export type ScrollBackground = 'white' | 'transparent'
export interface ScrollView {
  id: number; revision: number; collecting: boolean; archivePath: string | null;
  width: number; height: number; frames: number; message: string; preview: string; frameHeight: number
  canvasBounds: Rect; viewportRect: Rect; coveragePercent: number; hasHoles: boolean
  direction: 'none' | 'up' | 'down' | 'left' | 'right' | 'diagonal'
  status: 'collecting' | 'recovering' | 'finishing' | 'failed' 
}
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'hide' | 'close'
export type PinManageAction = 'show' | 'hide' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
export type AppDirectory = 'config' | 'data' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard' | 'logs'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
export const desktop = isTauri()
// 在唯一的 API 边界解码错误，截图、贴图与主页均能看到原生错误正文及失败操作。
async function invoke<T>(command: IpcCommand, args?: InvokeArgs): Promise<T> {
  try { return await nativeInvoke<T>(command, args) }
  catch (error) {
    const message = errorMessage(error)
    logIpcFailure(command, message, args && 'id' in args ? args.id : undefined)
    throw new Error(`${command}：${message}`)
  }
}
async function listen<T>(event: string, callback: EventCallback<T>, target?: string) {
  try { return await (target ? nativeListen<T>(event, callback, { target }) : nativeListen<T>(event, callback)) }
  catch (error) { throw new Error(`订阅 ${event}：${errorMessage(error)}`) }
}
export const api = {
  diagnosticsStatus: () => invoke<DiagnosticsStatus>('diagnostics_status'),
  diagnosticsRecentLogs: (query: LogQuery) => invoke<RecentLogs>('diagnostics_recent_logs', { query }),
  diagnosticsExport: () => invoke<boolean>('diagnostics_export'),
  diagnosticsClearArchives: () => invoke<ClearArchivesResult>('diagnostics_clear_archives'),
  diagnosticsSetDebug: (enabled: boolean) => invoke<DiagnosticsStatus>('diagnostics_set_debug', { enabled }),
  resourceStatus: () => invoke<ResourceStatus>('resource_status'),
  downloadResources: (kind: 'ocr' | 'font') => invoke<ResourceStatus>('download_resources', { kind }),
  chooseExportFont: () => invoke<ResourceStatus>('choose_export_font'),
  recognizeText: (id: number, rect: Rect, annotations: Annotation[]) => invoke<string>('recognize_capture_text', { id, rect, annotations }),
  recognizeCapture: (id: number, rect: Rect) => invoke<OcrResult>('recognize_capture_v2', { id, rect }),
  translateText: (requestId: string, text: string, consent: boolean, expectedSettings: AiSettings) => invoke<TranslationResult>('translate_text', { requestId, text, consent, expectedSettings }),
  translationModelStatus: () => invoke<LocalTranslationStatus>('translation_model_status'),
  translationModelAction: (modelId: string, action: 'download' | 'import' | 'delete') => invoke<LocalTranslationStatus>('translation_model_action', { modelId, action }),
  aiTranslationConfig: () => invoke<AiSettings>('ai_translation_config'),
  cancelAiJob: (requestId: string) => invoke<void>('cancel_ai_job', { requestId }),
  aiCredentialSet: (secret: string | null) => invoke<void>('ai_credential_set', { secret }),
  aiProviderStatus: () => invoke<boolean>('ai_provider_status'),
  aiProviderTest: () => invoke<void>('ai_provider_test'),
  copyOcrText: (text: string) => invoke<void>('copy_ocr_text', { text }),
  clipboardStatus: () => invoke<ClipboardStatus>('clipboard_status'),
  clipboardList: (query: ClipboardQuery) => invoke<ClipboardPage>('clipboard_list', { query }),
  clipboardText: (id: number) => invoke<string>('clipboard_get_text', { id }),
  clipboardThumbnail: (id: number) => invoke<string>('clipboard_get_thumbnail', { id }),
  clipboardRestore: (id: number) => invoke<void>('clipboard_restore', { id }),
  clipboardPin: (id: number) => invoke<void>('clipboard_pin_item', { id }),
  clipboardFavorite: (id: number, favorite: boolean) => invoke<void>('clipboard_set_favorite', { id, favorite }),
  clipboardDelete: (ids: number[]) => invoke<void>('clipboard_delete', { ids }),
  clipboardClear: (includeFavorites: boolean) => invoke<void>('clipboard_clear', { includeFavorites }),
  clipboardConfigure: (settings: ClipboardSettings) => invoke<ClipboardStatus>('clipboard_configure', { settings }),
  clipboardExport: (options: ClipboardExportOptions) => invoke<ClipboardExportResult>('clipboard_export', { options }),
  prepareRecordingEditor: (id: number) => invoke<RecordingEditorView>('prepare_recording_editor', { id }),
  closeRecordingEditor: (token: number) => invoke<void>('close_recording_editor', { token }),
  trimRecording: (token: number, range: RecordingTrimRange) => invoke<RecordingTrimExport | null>('trim_recording', { token, range }),
  cancelRecordingTrim: (token: number) => invoke<void>('cancel_recording_trim', { token }),
  latestRecordingResult: () => invoke<RecordingResult | null>('latest_recording_result'),
  recordingResultAction: (id: number, action: RecordingResultAction) => invoke<void>('recording_result_action', { id, action }),
  autostartEnabled: () => invoke<boolean>('autostart_enabled'),
  setAutostartEnabled: (enabled: boolean) => invoke<boolean>('set_autostart_enabled', { enabled }),
  recognizeQr: (id: number, rect: Rect, annotations: Annotation[]) => invoke<QrResult>('recognize_capture_qr', { id, rect, annotations }),
  recognizePinText: () => invoke<OcrResult>('recognize_pin_text'),
  recognizePinQr: () => invoke<QrResult>('recognize_pin_qr'),
  pinQrAction: (id: number, index: number, action: 'open' | 'copy') => invoke<void>('pin_qr_action', { id, index, action }),
  qrAction: (id: number, index: number, action: 'open' | 'copy') => invoke<void>('qr_action', { id, index, action }),
  completeOnboarding: () => invoke<Settings>('complete_onboarding'),
  appDirectories: () => invoke<DirectoryView[]>('app_directories'),
  openAppDirectory: (kind: AppDirectory) => invoke<void>('open_app_directory', { kind }),
  copyAppDirectory: (kind: AppDirectory) => invoke<void>('copy_app_directory', { kind }),
  copyShortcutExample: () => invoke<void>('copy_shortcut_example'),
  exportSettings: () => invoke<boolean>('export_settings'),
  selectRecordingArea: () => invoke<void>('select_recording_area'),
  recoverableRecordings: () => invoke<RecoverableRecording[]>('recoverable_recordings'),
  retryRecordingExport: (id: string) => invoke<string>('retry_recording_export', { id }),
  discardRecordingRecovery: (id: string) => invoke<void>('discard_recording_recovery', { id }),
  openProjectHomepage: () => invoke<void>('open_project_homepage'),
  status: () => invoke<AppStatus>('app_status'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  setShortcutsEnabled: (enabled: boolean) => invoke<Settings>('set_shortcuts_enabled', { enabled }),
  setShortcutEditing: (editing: boolean) => invoke<void>('set_shortcut_editing', { editing }),
  probeShortcuts: () => invoke<ShortcutProbeReport>('probe_shortcuts'),
  recoveryStatus: () => invoke<RecoveryStatus>('recovery_status'),
  recoveryArtifactAction: (id: string, action: RecoveryArtifactAction) => invoke<string | null>('recovery_artifact_action', { id, action }),
  cancelPending: () => invoke<void>('cancel_pending_capture'),
  start: () => invoke<void>('start_capture'),
  nativeWindow: (id: number, target: number) => invoke<CaptureFrame>('capture_native_window', { id, target }),
  current: () => invoke<CaptureFrame | null>('current_capture'),
  ready: (id: number) => invoke<void>('show_capture', { id }),
  cancel: (id: number) => invoke<void>('cancel_capture', { id }),
  copy: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('copy_capture', { id, rect, annotations }),
  copyColor: (id: number, point: Point, format: ColorFormat) => invoke<void>('copy_capture_color', { id, point, format }),
  save: (id: number, rect: Rect, annotations: Annotation[]) => invoke<boolean>('save_capture', { id, rect, annotations }),
  pin: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('pin_capture', { id, rect, annotations }),
  startScroll: (id: number, rect: Rect, saveFrames: boolean, mode: ScrollMode = 'auto', background: ScrollBackground = 'white') => invoke<ScrollView>('start_scroll', { id, rect, saveFrames, mode, background }),
  controlScroll: (id: number, cancel = false, confirmedOnly = false) => invoke<void>('control_scroll', { id, cancel, confirmedOnly }),
  currentScroll: () => invoke<ScrollView | null>('current_scroll'),
  startRecording: (id: number, rect: Rect, config?: RecorderConfig, target?: number, annotations?: Annotation[]) => invoke<void>('start_recording', { id, rect, config, ...(target === undefined ? {} : { target }), ...(annotations?.length ? { annotations } : {}) }),
  recordingCapabilities: () => invoke<RecorderCapabilities>('recording_capabilities'),
  currentRecordingDirectory: () => invoke<RecordingDirectory>('current_recording_directory'),
  chooseRecordingDirectory: (reset = false) => invoke<RecordingDirectory | null>('choose_recording_directory', { reset }),
  currentRecording: () => invoke<RecordingView | null>('current_recording'),
  recordingAnnotations: (knownRevision?: number) => invoke<RecordingAnnotationView | null>('current_recording_annotations', { knownRevision: knownRevision ?? null }),
  setRecordingAnnotationMode: (id: number, editing: boolean) => invoke<void>('set_recording_annotation_mode', { id, editing }),
  updateRecordingAnnotations: (id: number, shapes: Annotation[]) => invoke<RecordingAnnotationView>('update_recording_annotations', { id, shapes }),
  controlRecording: (id: number, action: RecordingAction) => invoke<void>('control_recording', { id, action }),
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
  hide: () => invoke<void>('hide_main'),
}
export const onFrame = (fn: (frame: CaptureFrame) => void) => listen<CaptureFrame>('capture-ready', e => fn(e.payload))
export const onRecordingPreviewClosed = (fn: (id: number) => void) => listen<number>('recording-preview-closed', e => fn(e.payload), 'recording-controls')
// 原生关闭/取消或采集接管选区后清空常驻 WebView 图片；编号用于过滤迟到事件。
// 接管时只发给 capture；滚动会话真正结束时再发给两个滚动辅助窗口。
// Tauri 的默认 Any 监听也会收到 emit_to 发往其他窗口的事件。
// 接管选区仅关闭 capture 浮层，不能让滚动预览将同编号会话永久判为已关闭。
export const onCaptureClosed = (fn: (id: number) => void, target: 'capture' | 'scroll-preview' | 'scroll-controls') =>
  listen<number>('capture-closed', e => fn(e.payload), target)
export const onResourceProgress = (fn: (progress: ResourceProgress) => void) => listen<ResourceProgress>('resource-download-progress', e => fn(e.payload))
// 原生入口只发会话 ID，选区与标注由截图窗口提交，避免读取旧剪贴板。
export const onCapturePin = (fn: (id: number) => void) => listen<number>('capture-pin-requested', e => fn(e.payload))
export const onError = (fn: (error: string) => void) => listen<string>('capture-error', e => fn(e.payload))
export const onResult = (fn: (result: string) => void) => listen<string>('capture-result', e => fn(e.payload))
export const onDelay = (fn: (remaining: number) => void) => listen<number>('capture-delay', e => fn(e.payload))
export const onPinsChanged = (fn: () => void) => listen('pins-changed', fn)
export const onSettingsChanged = (fn: () => void) => listen('settings-changed', fn)
export const onScroll = (fn: (view: ScrollView) => void) => listen<ScrollView>('scroll-updated', e => fn(e.payload))

export const onClipboardChanged = (fn: () => void) => listen('clipboard-changed', fn)
export const onClipboardError = (fn: (error: string) => void) => listen<string>('clipboard-error', e => fn(e.payload))
export const onClipboardState = (fn: (state: ClipboardStatus) => void) => listen<ClipboardStatus>('clipboard-state', e => fn(e.payload))
export const onClipboardOpen = (fn: () => void) => listen('clipboard-open', fn)
