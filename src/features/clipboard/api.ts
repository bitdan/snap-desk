/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { ClipboardSettings, ClipboardQuery, ClipboardPage, ClipboardStatus, ClipboardExportOptions, ClipboardExportResult } from '@/features/clipboard/clipboard'
import { invoke } from '@/services/ipc'

export const clipboardApi = {
  clipboardDrag: (id: number) => invoke<void>('clipboard_drag_item', { id }),
  clipboardStatus: () => invoke<ClipboardStatus>('clipboard_status'),
  clipboardList: (query: ClipboardQuery) => invoke<ClipboardPage>('clipboard_list', { query }),
  clipboardText: (id: number) => invoke<string>('clipboard_get_text', { id }),
  clipboardOpenLink: (id: number) => invoke<void>('clipboard_open_link', { id }),
  clipboardOpenPath: (id: number) => invoke<void>('clipboard_open_path', { id }),
  clipboardThumbnail: (id: number) => invoke<string>('clipboard_get_thumbnail', { id }),
  clipboardImage: (id: number) => invoke<string>('clipboard_get_image', { id }),
  clipboardRestore: (id: number) => invoke<void>('clipboard_restore', { id }),
  /** 写回剪贴板并粘贴到唤出前的窗口；返回 false 表示没有可粘贴的外部窗口，内容仅已复制。 */
  clipboardPaste: (id: number, keepOpen: boolean) => invoke<boolean>('clipboard_paste', { id, keepOpen }),
  clipboardPin: (id: number) => invoke<void>('clipboard_pin_item', { id }),
  clipboardFavorite: (id: number, favorite: boolean) => invoke<void>('clipboard_set_favorite', { id, favorite }),
  clipboardDelete: (ids: number[]) => invoke<void>('clipboard_delete', { ids }),
  clipboardClear: (includeFavorites: boolean) => invoke<void>('clipboard_clear', { includeFavorites }),
  clipboardSetEnabled: (enabled: boolean) => invoke<ClipboardStatus>('clipboard_set_enabled', { enabled }),
  clipboardWindow: (action: 'open' | 'hide' | 'drag') => invoke<void>('clipboard_window_control', { action }),
  clipboardConfigure: (settings: ClipboardSettings) => invoke<ClipboardStatus>('clipboard_configure', { settings }),
  clipboardExport: (options: ClipboardExportOptions) => invoke<ClipboardExportResult>('clipboard_export', { options }),
}
