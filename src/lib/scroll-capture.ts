import type { ScrollView } from './api'

/** 同一会话只接受更新版本，防止“开始”响应迟于“已经暂停”事件到达。 */
export function newerScroll(current: ScrollView | null, next: ScrollView, captureId: number | undefined): ScrollView | null {
  if (next.id !== captureId) return current
  if (current?.id === next.id && current.revision > next.revision) return current
  return next
}
