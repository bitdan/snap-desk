/** 本功能只配置视图；素材操作仍通过剪贴板命令按 ID 执行。 */
import { invoke } from '@/services/ipc'
import type { ShelfAction, ShelfSettings, ShelfSnapshot } from './types'
export const shelfApi = {
  shelfSnapshot: () => invoke<ShelfSnapshot>('shelf_snapshot'),
  shelfConfigure: (settings: ShelfSettings) => invoke<void>('shelf_configure', { settings }),
  shelfVisibility: (visible: boolean) => invoke<void>('shelf_visibility', { visible }),
  shelfItemAction: (id: number, action: ShelfAction) => invoke<void>('shelf_item_action', { id, action }),
}
