/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Annotation } from '@/features/annotations/annotations'
import type { Settings } from '@/features/settings/settings'
export interface PinAppearance { shadow: 'auto' | 'soft' | 'none'; cornerRadius: number; borderColor: string }
export interface PinView { clickThrough?: boolean; zoom: number; opacity: number; locked: boolean; topmost: boolean; toolbarExpanded: boolean }
/** 原生只回元信息；底图按 label 与 revision 经只读协议读取。 */
export interface PinFrame { label: string; revision: number; width: number; height: number; view: PinView; appearance: PinAppearance; transparencySupported: boolean; annotations: Annotation[] }
/** 前端视图状态：src 为空表示窗口长期隐藏后已释放底图。 */
export interface PinImage extends PinFrame { src: string }
export type PinTransform = 'rotateRight' | 'rotateLeft' | 'flipHorizontal' | 'flipVertical' | 'grayscale' | 'invert' | 'reset'
export type PinAction = 'copy' | 'save' | 'close'
// 字段与 Rust Settings / AppStatus 的 camelCase 序列化契约保持一致。
