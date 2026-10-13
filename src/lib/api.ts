import { invoke as nativeInvoke, isTauri, type InvokeArgs } from '@tauri-apps/api/core'
import { listen as nativeListen, type EventCallback } from '@tauri-apps/api/event'
import { errorMessage } from './errors'
import type { Point, Rect } from './selection'
import type { Annotation } from './annotations'
import type { Settings } from './settings'
import type { RecorderCapabilities, RecorderConfig, RecordingAction, RecordingView } from './recording'
export type { Settings } from './settings'

export interface CaptureFrame { id: number; width: number; height: number; dataUrl: string }
export interface PinView { zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
export interface PinFrame { label: string; width: number; height: number; dataUrl: string; view: PinView; transparencySupported: boolean; annotations: Annotation[] }
export interface PinSummary { label: string; width: number; height: number; visible: boolean; group: string }
export interface ColorPinRequest { color: string; width: number; height: number }
export interface TextPinRequest { background: string; annotation: Annotation }
export type ColorFormat = 'hex' | 'rgb'
export interface ScrollView {
  id: number; revision: number; collecting: boolean; automatic: boolean; archivePath: string | null;
  width: number; height: number; frames: number; message: string; preview: string; frameHeight: number
}
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'hide' | 'close'
export type PinManageAction = 'show' | 'hide' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
export interface AppStatus { settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
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
  status: () => invoke<AppStatus>('app_status'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  cancelPending: () => invoke<void>('cancel_pending_capture'),
  start: () => invoke<void>('start_capture'),
  current: () => invoke<CaptureFrame | null>('current_capture'),
  ready: (id: number) => invoke<void>('show_capture', { id }),
  cancel: (id: number) => invoke<void>('cancel_capture', { id }),
  copy: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('copy_capture', { id, rect, annotations }),
  copyColor: (id: number, point: Point, format: ColorFormat) => invoke<void>('copy_capture_color', { id, point, format }),
  save: (id: number, rect: Rect, annotations: Annotation[]) => invoke<boolean>('save_capture', { id, rect, annotations }),
  pin: (id: number, rect: Rect, annotations: Annotation[]) => invoke<void>('pin_capture', { id, rect, annotations }),
  startScroll: (id: number, rect: Rect, automatic: boolean, saveFrames: boolean) => invoke<ScrollView>('start_scroll', { id, rect, automatic, saveFrames }),
  controlScroll: (id: number, cancel = false) => invoke<void>('control_scroll', { id, cancel }),
  currentScroll: () => invoke<ScrollView | null>('current_scroll'),
  startRecording: (id: number, rect: Rect, config?: RecorderConfig) => invoke<void>('start_recording', { id, rect, config }),
  recordingCapabilities: () => invoke<RecorderCapabilities>('recording_capabilities'),
  currentRecording: () => invoke<RecordingView | null>('current_recording'),
  controlRecording: (id: number, action: RecordingAction) => invoke<void>('control_recording', { id, action }),
  pinClipboard: () => invoke<void>('pin_clipboard'),
  pinFile: () => invoke<boolean>('pin_file'),
  pinColor: (request: ColorPinRequest) => invoke<void>('pin_color', { request }),
  pinText: (request: TextPinRequest) => invoke<void>('pin_text', { request }),
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
export const onError = (fn: (error: string) => void) => listen<string>('capture-error', e => fn(e.payload))
export const onResult = (fn: (result: string) => void) => listen<string>('capture-result', e => fn(e.payload))
export const onDelay = (fn: (remaining: number) => void) => listen<number>('capture-delay', e => fn(e.payload))
export const onPinsChanged = (fn: () => void) => listen('pins-changed', fn)
export const onScroll = (fn: (view: ScrollView) => void) => listen<ScrollView>('scroll-updated', e => fn(e.payload))
