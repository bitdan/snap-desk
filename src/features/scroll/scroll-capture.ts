import type { ScrollView } from '@/features/scroll/types'

/** 四舍五入不能把仍有空洞的画布显示为已完整覆盖。 */
export function scrollCoverageLabel(view: Pick<ScrollView, 'coveragePercent' | 'hasHoles'>): string {
  const rounded = view.coveragePercent.toFixed(1)
  return view.hasHoles && rounded === '100.0' ? '<100' : rounded
}

/** 常驻预览窗口拒绝已关闭会话的迟到事件和查询响应，避免隐藏后重新持有图片。 */
export function newerScrollPreview(current: ScrollView | null, next: ScrollView, closedThroughId: number): ScrollView | null {
  if (next.id <= closedThroughId || (current && next.id < current.id)) return current
  return newerScroll(current, next, next.id)
}

/** 同一会话只接受更新版本，防止“开始”响应迟于“已经暂停”事件到达。 */
export function newerScroll(current: ScrollView | null, next: ScrollView, captureId: number | undefined): ScrollView | null {
  if (next.id !== captureId) return current
  if (current?.id === next.id && current.revision > next.revision) return current
  return next
}

/** 文档坐标可以为负，先减画布原点再归一化；裁到可见边界，避免框溢出。 */
export function scrollViewportStyle(view: Pick<ScrollView, 'canvasBounds' | 'viewportRect'>) {
  const b = view.canvasBounds, v = view.viewportRect
  if (b.width <= 0 || b.height <= 0) return { left: '0%', top: '0%', width: '0%', height: '0%' }
  const left = Math.max(0, Math.min(b.width, v.x - b.x))
  const top = Math.max(0, Math.min(b.height, v.y - b.y))
  const right = Math.max(left, Math.min(b.width, v.x + v.width - b.x))
  const bottom = Math.max(top, Math.min(b.height, v.y + v.height - b.y))
  return { left: `${left / b.width * 100}%`, top: `${top / b.height * 100}%`,
    width: `${(right - left) / b.width * 100}%`, height: `${(bottom - top) / b.height * 100}%` }
}
