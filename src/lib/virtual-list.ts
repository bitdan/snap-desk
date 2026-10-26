/** 前缀高度与二分查找支持不同高度的文本/图片/文件条目，不把列表强行裁成固定行高。 */
export function rowOffsets(heights: number[]): number[] {
  const offsets = [0]
  for (const height of heights) offsets.push(offsets.at(-1)! + Math.max(1, height))
  return offsets
}
export function rowAt(offsets: number[], position: number): number {
  let low = 0, high = Math.max(0, offsets.length - 2)
  while (low < high) {
    const mid = Math.ceil((low + high) / 2)
    if (offsets[mid] <= position) low = mid
    else high = mid - 1
  }
  return low
}
/** 预渲染上下各 320px；DOM 数量随视口大小变化，不随已加载历史总量增长。 */
export function visibleRows(offsets: number[], top: number, height: number, overscan = 320) {
  const count = offsets.length - 1
  if (!count) return { start: 0, end: 0, before: 0, after: 0 }
  const start = rowAt(offsets, Math.max(0, top - overscan))
  const end = Math.min(count, rowAt(offsets, top + height + overscan) + 1)
  return { start, end, before: offsets[start], after: offsets[count] - offsets[end] }
}
