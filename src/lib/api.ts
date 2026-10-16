import type { ClipboardSettings, ClipboardQuery, ClipboardPage, ClipboardStatus, ClipboardExportOptions, ClipboardExportResult } from './clipboard'
import { invoke as nativeInvoke, isTauri, type InvokeArgs } from '@tauri-apps/api/core'
import { listen as nativeListen, type EventCallback } from '@tauri-apps/api/event'
import { errorMessage } from './errors'
import type { Point, Rect } from './selection'
import type { Annotation } from './annotations'
import type { AiSettings, OcrResult, TranslationResult, LocalTranslationStatus } from './ai'
import type { Settings } from './settings'
import type { RecorderCapabilities, RecorderConfig, RecordingAction, RecordingDirectory, RecordingView, RecoverableRecording } from './recording'
import type { RecordingResult, RecordingResultAction } from './recording'
export type { Settings } from './settings'

export interface QrResult { id: number; items: { text: string; url: string | null }[] }
export interface ResourceStatus { directory: string; ocrReady: boolean; modelsReady: boolean; runtimeReady: boolean; runtimeDownloadMib: number; fontReady: boolean }
export interface ResourceProgress { name: string; received: number; total: number }
export interface CaptureFrame { id: number; width: number; height: number; dataUrl: string; recording: boolean }
export interface PinView { zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
export interface PinFrame { label: string; width: number; height: number; dataUrl: string; view: PinView; transparencySupported: boolean; annotations: Annotation[] }
export interface PinSummary { label: string; width: number; height: number; visible: boolean; group: string }
export type ColorFormat = 'hex' | 'rgb'
export interface ScrollView {
  id: number; revision: number; collecting: boolean; archivePath: string | null;
  width: number; height: number; frames: number; message: string; preview: string; frameHeight: number
}
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'hide' | 'close'
export type PinManageAction = 'show' | 'hide' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
export type AppDirectory = 'config' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
export const desktop = isTauri()
// 在唯一的 API 边界解码错误，截图、贴图与主页均能看到原生错误正文及失败操作。
async function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  try { return await nativeInvoke<T>(command, args) }
  catch (error) { throw new Error(`${command}：${errorMessage(error)}`) }
}
async function listen<T>(event: string, callback: EventCallback<T>) {
  try { return await nativeListen<T>(event, callback) }
  catch (error) { throw new Error(`订阅 ${event}：${errorMessage(error)}`) }
}
export const api = {
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
  latestRecordingResult: () => invoke<RecordingResult | null>('latest_recording_result'),
  recordingResultAction: (id: number, action: RecordingResultAction) => invoke<void>('recording_result_action', { id, action }),
  autostartEnabled: () => invoke<boolean>('autostart_enabled'),
  setAutostartEnabled: (enabled: boolean) => invoke<boolean>('set_autostart_enabled', { enabled }),
  recognizeQr: (id: number, rect: Rect, annotations: Annotation[]) => invoke<QrResult>('recognize_capture_qr', { id, rect, annotations }),
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
  openProjectHomepage: () => invoke<void>('open_project_homepage'),
  status: () => invoke<AppStatus>('app_status'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  setShortcutsEnabled: (enabled: boolean) => invoke<Settings>('set_shortcuts_enabled', { enabled }),
  setShortcutEditing: (editing: boolean) => invoke<void>('set_shortcut_editing', { editing }),
  cancelPending: () => invoke<void>('cancel_pending_capture'),
  start: () => invoke<void>('start_capture'),
  current: () => invoke<CaptureFrame | null>('current_capture'),
  ready: (id: number) => invoke<void>('show_capture', { id }),
  cancel: (id: number) => invoke<void>('cancel_capture', { id }),
  copy: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('copy_capture', { id, rect, annotations }),
  copyColor: (id: number, point: Point, format: ColorFormat) => invoke<void>('copy_capture_color', { id, point, format }),
  save: (id: number, rect: Rect, annotations: Annotation[]) => invoke<boolean>('save_capture', { id, rect, annotations }),
  pin: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('pin_capture', { id, rect, annotations }),
  startScroll: (id: number, rect: Rect, saveFrames: boolean) => invoke<ScrollView>('start_scroll', { id, rect, saveFrames }),
  controlScroll: (id: number, cancel = false) => invoke<void>('control_scroll', { id, cancel }),
  currentScroll: () => invoke<ScrollView | null>('current_scroll'),
  startRecording: (id: number, rect: Rect, config?: RecorderConfig) => invoke<void>('start_recording', { id, rect, config }),
  recordingCapabilities: () => invoke<RecorderCapabilities>('recording_capabilities'),
  currentRecordingDirectory: () => invoke<RecordingDirectory>('current_recording_directory'),
  chooseRecordingDirectory: (reset = false) => invoke<RecordingDirectory | null>('choose_recording_directory', { reset }),
  currentRecording: () => invoke<RecordingView | null>('current_recording'),
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
