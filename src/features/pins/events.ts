/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'

export const onPinsChanged = (fn: () => void) => listen('pins-changed', fn)
