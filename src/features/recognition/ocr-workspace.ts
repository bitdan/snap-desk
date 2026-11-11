/** 识别工作区的纯逻辑：文字整理、行定位与识别框坐标换算。 */
import type { OcrLine, OcrResult } from '@/features/recognition/ai'

/** 原图来源：截图传整屏图与选区，贴图传整张图；坐标均为图片像素。 */
export interface OcrImage { url: string; width: number; height: number; crop?: { x: number; y: number; width: number; height: number } }
export interface OcrBox { id: string; text: string; points: string }
export interface OcrImageView { url: string; width: number; height: number; viewBox: string; cropWidth: number; cropHeight: number; boxes: OcrBox[] }

/**
 * OCR 坐标相对识别输入（imageWidth × imageHeight），按比例映射到显示区域。
 * 宽高比不一致（如贴图旋转后识别的是原图）时只显示图片，不画错位的框。
 */
export function ocrImageView(image: OcrImage, result: OcrResult, lines: OcrLine[]): OcrImageView {
  const crop = image.crop ?? { x: 0, y: 0, width: image.width, height: image.height }
  const sourceWidth = result.imageWidth || crop.width, sourceHeight = result.imageHeight || crop.height
  const sx = crop.width / Math.max(1, sourceWidth), sy = crop.height / Math.max(1, sourceHeight)
  const aligned = Math.abs(sx - sy) <= Math.max(sx, sy) * 0.02
  const boxes = aligned ? lines.filter(line => line.polygon && line.polygon.length >= 3).map(line => ({
    id: line.id, text: line.text,
    points: line.polygon!.map(point => `${crop.x + point.x * sx},${crop.y + point.y * sy}`).join(' '),
  })) : []
  return { url: image.url, width: image.width, height: image.height, viewBox: `${crop.x} ${crop.y} ${crop.width} ${crop.height}`, cropWidth: crop.width, cropHeight: crop.height, boxes }
}

/** 原图按截图时的屏幕尺寸显示：物理像素除以缩放比即 CSS 像素，只缩小不放大。 */
export function ocrDisplaySize(image: OcrImage, scale: number): { width: number; height: number } {
  const crop = image.crop ?? { x: 0, y: 0, width: image.width, height: image.height }
  const ratio = Number.isFinite(scale) && scale > 0 ? scale : 1
  return { width: Math.max(1, Math.round(crop.width / ratio)), height: Math.max(1, Math.round(crop.height / ratio)) }
}
const MIN_HEIGHT = 620
/** 窗口尺寸跟随截图：原图按原尺寸放下，再加上右侧文字区与底栏，并限制在屏幕可用区域内。 */
export function ocrWindowSize(image: OcrImage | null, scale: number, available: { width: number; height: number }): { width: number; height: number } {
  const maxWidth = Math.max(640, Math.floor(available.width * 0.92)), maxHeight = Math.max(420, Math.floor(available.height * 0.88))
  if (!image) return { width: Math.min(720, maxWidth), height: Math.min(MIN_HEIGHT, maxHeight) }
  const size = ocrDisplaySize(image, scale)
  // 24 为原图区内边距，400 为文字区，52 为底栏，再留出系统标题栏高度。
  // 宽而矮的截图也保证右侧翻译视图的原文、语言条和译文有足够高度。
  return {
    width: Math.min(maxWidth, Math.max(720, size.width + 24 + 400)),
    height: Math.min(maxHeight, Math.max(MIN_HEIGHT, size.height + 24 + 20 + 52 + 32)),
  }
}

/** 按阅读顺序在当前文字里依次查找每行，用户改过的行找不到时跳过，不影响其余行定位。 */
export function lineRanges(text: string, lines: OcrLine[]): { id: string; start: number; end: number }[] {
  const ranges: { id: string; start: number; end: number }[] = []
  let cursor = 0
  for (const line of lines) {
    const value = line.text.trim()
    if (!value) continue
    const start = text.indexOf(value, cursor)
    if (start < 0) continue
    ranges.push({ id: line.id, start, end: start + value.length })
    cursor = start + value.length
  }
  return ranges
}

const cjk = '\\u2e80-\\u9fff\\uac00-\\ud7af\\uff00-\\uffef\\u3000-\\u303f'
/** 合并换行：英文连字符断词直接拼接，中文之间不加空格，其他用空格连接；空行保留为段落。 */
export function mergeLines(text: string): string {
  return text.replace(/\r\n?/g, '\n').split(/\n\s*\n/).map(paragraph => paragraph
    .replace(/-\s*\n\s*/g, '')
    .replace(new RegExp(`([${cjk}])\\s*\\n\\s*(?=[${cjk}])`, 'g'), '$1')
    .replace(/\s*\n\s*/g, ' ')
    .trim()).join('\n\n')
}
/** 识别结果进入可编辑草稿前的整理：设置开启“自动合并换行”时先合并，原始结果仍可一键还原。 */
export function initialOcrText(result: Pick<OcrResult, 'fullText' | 'mergeLines'>): string {
  const text = result.fullText ?? ''
  return result.mergeLines ? mergeLines(text) : text
}
/** 整理空格：去掉 OCR 常见的汉字间空格、行首尾空白，并把连续空格压成一个。 */
export function tidySpaces(text: string): string {
  return text.replace(/\r\n?/g, '\n').split('\n').map(line => line
    .replace(new RegExp(`([${cjk}])[ \\t]+(?=[${cjk}])`, 'g'), '$1')
    .replace(/[ \t]{2,}/g, ' ')
    .trim()).join('\n')
}
