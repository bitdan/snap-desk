/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */
import type { ResourceProgress } from '@/features/resources/types'
import { listen } from '@/services/ipc'

export const onResourceProgress = (fn: (progress: ResourceProgress) => void) => listen<ResourceProgress>('resource-download-progress', e => fn(e.payload))
