/** 历史更新、收藏和视图偏好变化统一触发刷新。 */
import { listen } from '@/services/ipc'
export const onShelfChanged = (callback: () => void) => listen('shelf-changed', callback)
export const onShelfRevealed = (callback: () => void) => listen('shelf-revealed', () => callback())
export const onShelfCaptureState = (callback: (suspended: boolean) => void) => listen<boolean>('shelf-capture-state', event => callback(event.payload))
