/** 与 Rust clipboard/model.rs、export.rs 的受限 IPC 契约一致。 */
export interface ClipboardSettings {
  enabled: boolean; maxItems: number; maxStorageMb: number; maxTextBytes: number;
  maxImagePixels: number; saveCopiedImages: boolean; saveCopiedText: boolean; saveCopiedFiles: boolean; retainDays: number;
}
export function defaultClipboardSettings(): ClipboardSettings {
  return { enabled: false, maxItems: 500, maxStorageMb: 200, maxTextBytes: 1048576, maxImagePixels: 16000000, saveCopiedImages: true, saveCopiedText: true, saveCopiedFiles: true, retainDays: 0 }
}
export type ClipboardKind = 'text' | 'image' | 'file'
export interface ClipboardItem {
  id: number; kind: ClipboardKind; source: 'clipboard' | 'capture' | 'scroll'; preview: string;
  mimeType: string; width: number | null; height: number | null; byteSize: number;
  favorite: boolean; createdAt: number; lastSeenAt: number; seenCount: number;
  filePaths?: string[];
}
export interface ClipboardQuery { keyword?: string; kind?: ClipboardKind; favoritesOnly?: boolean; limit?: number; cursor?: string }
export interface ClipboardPage { items: ClipboardItem[]; nextCursor: string | null; revision: number }
export interface ClipboardStatus { enabled: boolean; supported: boolean; count: number; byteSize: number; revision: number; error: string | null }
export interface ClipboardExportOptions { ids: number[]; mode: 'pdf' | 'longPng' | 'separate'; pageSize: 'A4' | 'original'; gapPx: number; marginMm: number; background: 'white' | 'transparent' }
export interface ClipboardExportResult { status: 'saved' | 'cancelled'; outputPaths: string[] }

/** 多选顺序就是导出顺序；删除后不把新列表的次序覆盖到用户的选择上。 */
export function toggleSelection(ids: number[], id: number): number[] {
  return ids.includes(id) ? ids.filter(value => value !== id) : ids.length < 100 ? [...ids, id] : ids
}

export interface ClipboardSelectionGesture { toggle?: boolean; range?: boolean }
/** 内容单击替换选择；复选框/Ctrl 保留顺序多选，Shift 按当前筛选列表连续选择。 */
export function selectClipboardItems(ids: number[], id: number, ordered: number[], anchor: number | null, gesture: ClipboardSelectionGesture): number[] {
  if (gesture.range && anchor !== null && ordered.includes(anchor) && ordered.includes(id)) {
    const start = ordered.indexOf(anchor), end = ordered.indexOf(id)
    const range = ordered.slice(Math.min(start, end), Math.max(start, end) + 1)
    return [...new Set([...(gesture.toggle ? ids : []), ...range])].slice(0, 100)
  }
  return gesture.toggle ? toggleSelection(ids, id) : [id]
}
export function reorderSelection(ids: number[], from: number, to: number): number[] {
  if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < 0 || from >= ids.length || to >= ids.length) return [...ids]
  const result = [...ids]; const [id] = result.splice(from, 1); result.splice(to, 0, id); return result
}
