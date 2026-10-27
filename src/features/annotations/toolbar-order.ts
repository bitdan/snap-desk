import { annotationTools } from '@/features/annotations/annotation-tools'
import type { AnnotationTool } from '@/features/annotations/use-annotations'

export const toolbarOrderKey = 'snap-desk.toolbar-order.v2'
/** 去掉旧工具、重复和非法值；新增箭头文字放在箭头右侧，其他新增工具补在末尾。 */
export function normalizeToolOrder(value: unknown): AnnotationTool[] {
  const defaults = annotationTools.map(item => item.tool)
  const saved = Array.isArray(value) ? value.filter((tool): tool is typeof defaults[number] => defaults.includes(tool)) : []
  const order = [...new Set([...saved, ...defaults])]
  if (!saved.includes('arrow-text')) placeArrowText(order)
  return order
}
/** 首次迁移旧顺序时调整相邻位置；之后仍允许用户自由拖动排序。 */
function placeArrowText(order: AnnotationTool[]) {
  order.splice(order.indexOf('arrow-text'), 1)
  order.splice(order.indexOf('arrow') + 1, 0, 'arrow-text')
}
export function readToolOrder(): AnnotationTool[] {
  try {
    const saved = localStorage.getItem(toolbarOrderKey)
    if (saved !== null) return normalizeToolOrder(JSON.parse(saved))
    const order = normalizeToolOrder(JSON.parse(localStorage.getItem('snap-desk.toolbar-order.v1') ?? 'null'))
    placeArrowText(order)
    return order
  }
  catch { return normalizeToolOrder(null) }
}
/** 移到目标所在位置，其余工具保持相对顺序。 */
export function moveTool(order: AnnotationTool[], source: AnnotationTool, target: AnnotationTool): AnnotationTool[] {
  const next = [...order], from = next.indexOf(source), to = next.indexOf(target)
  if (from < 0 || to < 0 || from === to) return next
  next.splice(from, 1); next.splice(to, 0, source)
  return next
}
