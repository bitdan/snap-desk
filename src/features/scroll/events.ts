/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */
import type { ScrollView } from '@/features/scroll/types'
import { listen } from '@/services/ipc'

export const onScroll = (fn: (view: ScrollView) => void) => listen<ScrollView>('scroll-updated', e => fn(e.payload))
