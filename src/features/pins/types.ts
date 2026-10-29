/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Annotation } from '@/features/annotations/annotations'
import type { Settings } from '@/features/settings/settings'
export interface PinAppearance { shadow: 'auto' | 'soft' | 'none'; cornerRadius: number; borderColor: string }
export interface PinView { zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
/** dataUrl 为空表示调用方已持有 revision 对应的底图；隐藏释放后须不带版本重新读取。 */
export interface PinFrame { label: string; revision: number; width: number; height: number; dataUrl: string; view: PinView; appearance: PinAppearance; transparencySupported: boolean; annotations: Annotation[] }
export interface PinSummary { label: string; width: number; height: number; visible: boolean; group: string; thumbnail: string }
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'hide' | 'close'
export type PinManageAction = 'show' | 'hide' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
