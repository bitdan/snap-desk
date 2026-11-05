/** 所有尺寸为 CSS 像素；透明窗口只安排一排卡片，不依赖浏览器滚动条。 */
export function shelfPhotoSize(width: number | null, height: number | null) {
  if (!width || !height || width <= 0 || height <= 0) return { width: 144, height: 96 }
  const scale = Math.min(144 / width, 108 / height)
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) }
}

/** 卡片夹子与绳索使用同一个高度函数，沿抛物线自然下垂。 */
export function shelfLayout(width: number, count: number) {
  const capacity = Math.max(1, Math.min(5, Math.floor((width - 80) / 174)))
  const visible = Math.min(capacity, count)
  const spacing = Math.min(174, Math.max(1, width - 48))
  const sag = Math.min(20, width * 0.024)
  const ropeY = (x: number) => 12 + 4 * sag * (x / width) * (1 - x / width)
  return {
    capacity,
    rope: `M 0 12 Q ${width / 2} ${12 + sag * 2} ${width} 12`,
    positions: Array.from({ length: visible }, (_, i) => {
      const x = width / 2 + (i - (visible - 1) / 2) * spacing
      return { left: x - 78, top: ropeY(x) - 9 }
    }),
  }
}
