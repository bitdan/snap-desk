/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { ClipboardSettings, ClipboardQuery, ClipboardPage, ClipboardStatus, ClipboardExportOptions, ClipboardExportResult } from '@/features/clipboard/clipboard'
import { invoke } from '@/services/ipc'

export const clipboardApi = {
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
}
