/** 暂存架与历史共享条目 ID；不另外持有图片或文件副本。 */
import type { ClipboardItem } from '@/features/clipboard/clipboard'
export type ShelfCategory = 'text' | 'image' | 'file' | 'link' | 'path' | 'color' | 'favorites'
export interface ShelfSettings { enabled: boolean; topHover: boolean; categories: ShelfCategory[] }
export interface ShelfSnapshot { settings: ShelfSettings; keptIds: number[]; items: ClipboardItem[]; recordingEnabled: boolean }
export type ShelfAction = 'keep' | 'unkeep' | 'remove' | 'restore'
export const shelfCategories: { key: ShelfCategory; label: string }[] = [
  { key: 'text', label: '文本' }, { key: 'image', label: '图片' }, { key: 'file', label: '文件' },
  { key: 'link', label: '链接' }, { key: 'path', label: '文件路径' }, { key: 'color', label: '颜色' }, { key: 'favorites', label: '收藏' },
]
export const defaultShelfSettings = (): ShelfSettings => ({ enabled: false, topHover: false, categories: ['image'] })
