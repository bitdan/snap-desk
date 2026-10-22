import { annotationTools } from './annotation-tools'
import type { AnnotationTool } from './use-annotations'

export const toolbarOrderKey = 'snap-desk.toolbar-order.v1'
/** 去掉旧工具、重复和非法值；新版本新增的工具自动补在末尾。 */
export function normalizeToolOrder(value: unknown): AnnotationTool[] {
  const defaults = annotationTools.map(item => item.tool)
  const saved = Array.isArray(value) ? value.filter((tool): tool is typeof defaults[number] => defaults.includes(tool)) : []
  return [...new Set([...saved, ...defaults])]
}
export function readToolOrder(): AnnotationTool[] {
  try { return normalizeToolOrder(JSON.parse(localStorage.getItem(toolbarOrderKey) ?? 'null')) }
  catch { return normalizeToolOrder(null) }
}
/** 移到目标所在位置，其余工具保持相对顺序。 */
export function moveTool(order: AnnotationTool[], source: AnnotationTool, target: AnnotationTool): AnnotationTool[] {
  const next = [...order], from = next.indexOf(source), to = next.indexOf(target)
  if (from < 0 || to < 0 || from === to) return next
  next.splice(from, 1); next.splice(to, 0, source)
  return next
}
