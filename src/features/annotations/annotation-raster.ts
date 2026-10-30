import { annotationTextLayout } from './annotation-text-layout'
import { isRaster, shapeBounds, magnifierSource, type Annotation } from '@/features/annotations/annotations'
import { clamp, type Rect } from '@/shared/geometry/selection'

const fonts = { sans: 'sans-serif', serif: 'serif', mono: 'monospace' }
/** 中文字形由当前 WebView 绘制为透明 PNG；预览/导出使用同一份字形，避免平台字体差异。 */
export async function prepareRaster(shape: Annotation, imageUrl: string, crop: Rect, preserveGeometry = false): Promise<Annotation> {
  if (!isRaster(shape)) return shape
  // 临时画布和解码图只属于本次生成；成功、解码失败、超限都走 finally 释放。
  const canvases: HTMLCanvasElement[] = [], images: HTMLImageElement[] = []
  const createCanvas = () => { const value = document.createElement('canvas'); canvases.push(value); return value }
  const createImage = () => { const value = new Image(); value.crossOrigin = 'anonymous'; images.push(value); return value }
  try {
    const canvas = createCanvas(), context = canvas.getContext('2d')!
    let box = shapeBounds(shape)
    if (shape.kind === 'text' && preserveGeometry && shape.raster) {
      // 改颜色时复用现有字形和透明度，旋转/翻转后的文字不会被重新排成水平行。
      const glyph = createImage(); glyph.src = `data:image/png;base64,${shape.raster}`; await glyph.decode()
      canvas.width = glyph.naturalWidth; canvas.height = glyph.naturalHeight
      context.drawImage(glyph, 0, 0); context.globalCompositeOperation = 'source-in'; context.fillStyle = shape.color
      context.fillRect(0, 0, canvas.width, canvas.height)
      const raster = canvas.toDataURL('image/png').split(',')[1]
      if (raster.length > 2_000_000) throw new Error('此标注图片过大，请缩小区域。')
      return { ...shape, raster }
    }
    if (shape.kind === 'number') {
      if (!/^[1-9]\d{0,2}$/.test(shape.text ?? '')) throw new Error('序号须为 1～999。')
      // 固定高分辨率字形保证缩放清晰；几何范围保持不变，预览与原生导出共用 PNG。
      canvas.width = 144; canvas.height = 144
      await document.fonts.ready
      context.fillStyle = shape.color
      context.beginPath(); context.arc(72, 72, 70, 0, Math.PI * 2); context.fill()
      context.font = `bold ${shape.text!.length === 3 ? 60 : 76}px sans-serif`
      const rgb = [1, 3, 5].map(offset => parseInt(shape.color.slice(offset, offset + 2), 16))
      context.fillStyle = rgb[0] * .299 + rgb[1] * .587 + rgb[2] * .114 > 160 ? '#111827' : '#ffffff'
      context.textAlign = 'center'; context.textBaseline = 'middle'
      context.fillText(shape.text!, 72, 76)
    } else if (shape.kind === 'text') {
      const size = shape.fontSize ?? 24
      if (!shape.text?.trim()) throw new Error('文字不能为空。')
      const font = `${size}px ${fonts[shape.fontFamily ?? 'sans']}`
      await document.fonts.ready
      context.font = font
      const { lines, width, height } = annotationTextLayout(shape.text, size, crop.width, text => context.measureText(text).width)
      if (height > crop.height) throw new Error('文字超出区域，请减少文字、缩小字号或扩大选区。')
      // 保持原图上的文字尺寸；只降低超大字形 PNG 的采样率，满足原生 2048 边长和 1MP 限额。
      const scale = Math.min(1, 2048 / height, Math.sqrt(1_048_576 / (width * height)))
      canvas.width = Math.max(1, Math.floor(width * scale)); canvas.height = Math.max(1, Math.floor(height * scale))
      if (scale < 1) context.scale(scale, scale)
      context.font = font; context.fillStyle = shape.color; context.textBaseline = 'top'
      lines.forEach((line, i) => context.fillText(line, 2, 2 + i * size * 1.3))
      // 新建/改文字时重新排版；纯几何缩放直接复用字形 PNG。
      if (!preserveGeometry) box = { x: clamp(box.x, crop.x, crop.x + crop.width - width), y: clamp(box.y, crop.y, crop.y + crop.height - height), width, height }
    } else {
      const image = createImage()
      image.src = imageUrl
      await image.decode()
      const scale = Math.min(1, 1024 / Math.max(box.width, box.height))
      canvas.width = Math.max(1, Math.ceil(box.width * scale)); canvas.height = Math.max(1, Math.ceil(box.height * scale))
      if (shape.kind === 'magnify') {
        const source = magnifierSource(shape)
        context.imageSmoothingEnabled = true
        context.drawImage(image, source.x, source.y, source.width, source.height, 0, 0, canvas.width, canvas.height)
        // 边框画在镜框内部，避免导出时越界；只放大底图，不合并其他标注。
        const border = Math.min(shape.width * scale, canvas.width / 2, canvas.height / 2)
        context.strokeStyle = shape.color; context.lineWidth = border
        context.strokeRect(border / 2, border / 2, canvas.width - border, canvas.height - border)
      } else if (shape.kind === 'blur') {
        // 扩展边缘后再模糊，避免透明的模糊边缘露出底下未处理的原图。
        const radius = Math.max(2, shape.width * scale), pad = Math.ceil(radius * 3)
        const padded = createCanvas(), w = canvas.width, h = canvas.height
        padded.width = w + pad * 2; padded.height = h + pad * 2
        const ctx = padded.getContext('2d')!
        ctx.drawImage(image, box.x, box.y, box.width, box.height, pad, pad, w, h)
        ctx.drawImage(padded, pad, pad, 1, h, 0, pad, pad, h)
        ctx.drawImage(padded, pad + w - 1, pad, 1, h, pad + w, pad, pad, h)
        ctx.drawImage(padded, 0, pad, padded.width, 1, 0, 0, padded.width, pad)
        ctx.drawImage(padded, 0, pad + h - 1, padded.width, 1, 0, pad + h, padded.width, pad)
        context.filter = `blur(${radius}px)`
        context.drawImage(padded, -pad, -pad)
      } else {
        // 先缩至像素块网格，再用最近邻放大；透明度固定为 1，避免敏感内容透出。
        const small = createCanvas()
        small.width = Math.max(1, Math.ceil(box.width / Math.max(4, shape.width)))
        small.height = Math.max(1, Math.ceil(box.height / Math.max(4, shape.width)))
        small.getContext('2d')!.drawImage(image, box.x, box.y, box.width, box.height, 0, 0, small.width, small.height)
        context.imageSmoothingEnabled = false
        context.drawImage(small, 0, 0, canvas.width, canvas.height)
      }
    }
    const raster = canvas.toDataURL('image/png').split(',')[1]
    if (raster.length > 2_000_000) throw new Error('此标注图片过大，请缩小区域。')
    return { ...shape, start: { x: box.x, y: box.y }, end: { x: box.x + box.width, y: box.y + box.height }, raster,
      opacity: ['text', 'number', 'magnify'].includes(shape.kind) ? shape.opacity ?? 1 : 1 }
  } finally {
    // 清空 src 解除 Data URL/解码像素引用；缩为 0 立即丢弃画布后备缓冲。
    for (const image of images) image.removeAttribute('src')
    for (const canvas of canvases) { canvas.width = 0; canvas.height = 0 }
  }
}
