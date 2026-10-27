import type { Rect, Size } from '@/shared/geometry/selection'

/** 只记住最后一次成功导出的范围，不保存截图内容或标注。 */
export interface RecentSelection { version: 1; image: Size & { layoutKey?: string }; rect: Rect }
const storageKey = 'snap-desk.recent-selection.v1'

function positiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
}

/** 本地存储可能被旧版本或外部修改，恢复前重新验证完整的像素边界。 */
export function parseRecentSelection(raw: string | null): RecentSelection | null {
  try {
    const value = JSON.parse(raw ?? 'null')
    if (!value || value.version !== 1) return null
    const { image, rect } = value
    if (!image || !rect || !positiveInteger(image.width) || !positiveInteger(image.height)
      || !positiveInteger(rect.width) || !positiveInteger(rect.height)
      || !Number.isSafeInteger(rect.x) || !Number.isSafeInteger(rect.y) || rect.x < 0 || rect.y < 0
      || rect.x + rect.width > image.width || rect.y + rect.height > image.height) return null
    return { version: 1, image: { width: image.width, height: image.height, ...(typeof image.layoutKey === 'string' ? { layoutKey: image.layoutKey } : {}) },
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } }
  } catch { return null }
}

/** 两轴分别换算 CSS 坐标；分辨率变化时不猜测原来的目标位置。 */
export function restoreRecentSelection(recent: RecentSelection | null, image: Size & { layoutKey?: string }, viewport: Size): Rect | null {
  if (!recent || recent.image.layoutKey !== image.layoutKey || recent.image.width !== image.width || recent.image.height !== image.height
    || !Number.isFinite(viewport.width) || !Number.isFinite(viewport.height)
    || viewport.width <= 0 || viewport.height <= 0) return null
  const { rect } = recent
  return { x: rect.x * viewport.width / image.width, y: rect.y * viewport.height / image.height,
    width: rect.width * viewport.width / image.width, height: rect.height * viewport.height / image.height }
}

/** WebView 禁用存储或配额不足不应影响截图；仍可在当前窗口会话中记住范围。 */
export function readRecentSelection(): RecentSelection | null {
  try { return parseRecentSelection(localStorage.getItem(storageKey)) } catch { return null }
}
export function writeRecentSelection(rect: Rect, image: Size & { layoutKey?: string }): RecentSelection | null {
  // 显式拣选尺寸字段，避免把 CaptureFrame 的其他会话字段 一并写入存储。
  const recent = parseRecentSelection(JSON.stringify({ version: 1,
    image: { width: image.width, height: image.height, ...(typeof image.layoutKey === 'string' ? { layoutKey: image.layoutKey } : {}) }, rect }))
  if (recent) {
    try { localStorage.setItem(storageKey, JSON.stringify(recent)) } catch { /* 保留内存回退。 */ }
  }
  return recent
}
