/** 原生事件订阅；返回取消监听函数，由窗口卸载时释放。 */

import { listen } from '@/services/ipc'

/** 只通知状态已变化，窗口按自身权限查询快照，不广播路径或标注大数据。 */
export const onRecordingChanged = (fn: () => void) => listen<null>('recording-changed', fn)

export const onRecordingPreviewClosed = (fn: (id: number) => void) => listen<number>('recording-preview-closed', e => fn(e.payload), 'recording-controls')
