/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Annotation } from '@/features/annotations/annotations'
import type { Settings } from '@/features/settings/settings'
export interface PinView { zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
export interface PinFrame { label: string; width: number; height: number; dataUrl: string; view: PinView; transparencySupported: boolean; annotations: Annotation[] }
export interface PinSummary { label: string; width: number; height: number; visible: boolean; group: string }
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'hide' | 'close'
export type PinManageAction = 'show' | 'hide' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
