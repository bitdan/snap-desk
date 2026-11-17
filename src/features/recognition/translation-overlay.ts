/** 截图翻译覆盖原文的纯逻辑：识别行合并为段落、取色与译文排版。坐标均为识别输入像素。 */
import type { OcrLine, OcrResult } from '@/features/recognition/ai'
import type { OcrImage } from '@/features/recognition/ocr-workspace'
import { mergeLines } from '@/features/recognition/ocr-workspace'

export interface OverlayBox { x: number; y: number; width: number; height: number }
export interface OverlayParagraph { ids: string[]; text: string; box: OverlayBox; lineHeight: number }
export interface OverlayColors { background: string; foreground: string }
export interface OverlayLayout { fontSize: number; lines: string[] }
export interface PixelData { data: ArrayLike<number>; width: number; height: number }

/** 批量翻译最多 50 段；超出时退回普通翻译，不截断原文。 */
const MAX_PARAGRAPHS = 50
/** 倾斜超过约 15° 的行（竖排、旋转文字）无法用水平文本覆盖，整体退回普通翻译。 */
const MAX_TILT = Math.tan(15 * Math.PI / 180)

function lineBox(line: OcrLine): (OverlayBox & { tilt: number }) | null {
  const points = line.polygon
  if (!points || points.length < 3 || points.some(p => !Number.isFinite(p.x) || !Number.isFinite(p.y))) return null
  const xs = points.map(p => p.x), ys = points.map(p => p.y)
  const x = Math.min(...xs), y = Math.min(...ys)
  const dx = points[1].x - points[0].x, dy = points[1].y - points[0].y
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y, tilt: dx > 0 ? Math.abs(dy / dx) : Infinity }
}

/**
 * 按阅读顺序把相邻行合成段落：行高相近、上下间距小且水平方向重叠。
 * 逐行翻译会把句子拆碎，按段落翻译语义更完整；任一行缺少坐标或明显倾斜时返回 null。
 */
export function overlayParagraphs(lines: OcrLine[]): OverlayParagraph[] | null {
  const sorted = lines.filter(line => line.text.trim()).sort((a, b) => a.order - b.order)
  if (!sorted.length) return null
  const groups: { line: OcrLine; box: OverlayBox }[][] = []
  for (const line of sorted) {
    const box = lineBox(line)
    if (!box || box.height <= 0 || box.width <= 0 || box.tilt > MAX_TILT) return null
    const group = groups.at(-1), last = group?.at(-1)?.box
    if (group && last) {
      const small = Math.min(last.height, box.height), large = Math.max(last.height, box.height)
      const gap = box.y - (last.y + last.height)
      const overlap = Math.min(last.x + last.width, box.x + box.width) - Math.max(last.x, box.x)
      if (large / small <= 1.6 && gap <= small * 0.8 && gap >= -small * 0.5 && overlap > -small) { group.push({ line, box }); continue }
    }
    groups.push([{ line, box }])
  }
  if (groups.length > MAX_PARAGRAPHS) return null
  return groups.map(group => {
    const x = Math.min(...group.map(item => item.box.x)), y = Math.min(...group.map(item => item.box.y))
    const right = Math.max(...group.map(item => item.box.x + item.box.width)), bottom = Math.max(...group.map(item => item.box.y + item.box.height))
    const heights = group.map(item => item.box.height).sort((a, b) => a - b)
    return {
      ids: group.map(item => item.line.id),
      text: mergeLines(group.map(item => item.line.text.trim()).join('\n')),
      box: { x, y, width: right - x, height: bottom - y },
      lineHeight: heights[Math.floor(heights.length / 2)],
    }
  })
}

/** 用户在翻译视图里改过原文后，段落译文不再对应原图；忽略空白和连字符（合并行会去掉行尾断词连字符）。 */
export function matchesSegments(text: string, segments: string[] | null | undefined): segments is string[] {
  if (!segments?.length) return false
  const compact = (value: string) => value.replace(/[\s\-‐‑]+/g, '')
  return compact(text) === compact(segments.join(''))
}

/** 识别坐标映射到原图显示坐标（与识别框一致）；宽高比不一致时不覆盖，避免错位。 */
export function overlayBoxes(image: OcrImage, result: Pick<OcrResult, 'imageWidth' | 'imageHeight'>, paragraphs: OverlayParagraph[]): OverlayBox[] | null {
  const crop = image.crop ?? { x: 0, y: 0, width: image.width, height: image.height }
  const sx = crop.width / Math.max(1, result.imageWidth || crop.width), sy = crop.height / Math.max(1, result.imageHeight || crop.height)
  if (Math.abs(sx - sy) > Math.max(sx, sy) * 0.02) return null
  return paragraphs.map(({ box }) => ({ x: crop.x + box.x * sx, y: crop.y + box.y * sy, width: box.width * sx, height: box.height * sy }))
}

function rgb(r: number, g: number, b: number) { return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})` }

/**
 * 背景取段落外沿一圈最常见的颜色（量化后取众数再求均值），可抵抗相邻文字和抗锯齿的干扰；
 * 文字色取框内与背景差异最大的那部分像素的均值。像素与 box 使用同一坐标系。
 */
export function sampleColors(pixels: PixelData, box: OverlayBox): OverlayColors {
  const { data, width, height } = pixels
  const pad = Math.max(2, Math.round(Math.min(box.width, box.height) * 0.15))
  const left = Math.max(0, Math.floor(box.x) - pad), top = Math.max(0, Math.floor(box.y) - pad)
  const right = Math.min(width - 1, Math.ceil(box.x + box.width) + pad - 1), bottom = Math.min(height - 1, Math.ceil(box.y + box.height) + pad - 1)
  const buckets = new Map<number, [number, number, number, number]>()
  const add = (x: number, y: number) => {
    const i = (y * width + x) * 4, r = data[i], g = data[i + 1], b = data[i + 2]
    const key = (r >> 4) << 8 | (g >> 4) << 4 | b >> 4
    const bucket = buckets.get(key) ?? [0, 0, 0, 0]
    bucket[0] += r; bucket[1] += g; bucket[2] += b; bucket[3]++
    buckets.set(key, bucket)
  }
  for (let x = left; x <= right; x++) { add(x, top); if (bottom !== top) add(x, bottom) }
  for (let y = top + 1; y < bottom; y++) { add(left, y); if (right !== left) add(right, y) }
  const best = buckets.size ? [...buckets.values()].reduce((a, b) => b[3] > a[3] ? b : a) : [255, 255, 255, 1]
  const bg = [best[0] / best[3], best[1] / best[3], best[2] / best[3]]

  // 两遍扫描：先求最大色差，再对色差较大的“墨迹”像素求均值，避免为大框分配逐像素对象。
  const x0 = Math.max(0, Math.floor(box.x)), y0 = Math.max(0, Math.floor(box.y))
  const x1 = Math.min(width - 1, Math.ceil(box.x + box.width) - 1), y1 = Math.min(height - 1, Math.ceil(box.y + box.height) - 1)
  const distance = (i: number) => Math.hypot(data[i] - bg[0], data[i + 1] - bg[1], data[i + 2] - bg[2])
  let maxDistance = 0
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) maxDistance = Math.max(maxDistance, distance((y * width + x) * 4))
  const background = rgb(bg[0], bg[1], bg[2])
  // 框内几乎没有对比（取样失败或纯色块）时按背景亮度给出可读的黑或白。
  if (maxDistance < 48) return { background, foreground: 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2] > 140 ? 'rgb(17, 17, 17)' : 'rgb(245, 245, 245)' }
  const sum = [0, 0, 0]
  let count = 0
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) {
    const i = (y * width + x) * 4
    if (distance(i) < maxDistance * 0.6) continue
    sum[0] += data[i]; sum[1] += data[i + 1]; sum[2] += data[i + 2]; count++
  }
  return { background, foreground: rgb(sum[0] / count, sum[1] / count, sum[2] / count) }
}

const cjk = /[⺀-鿿가-힯＀-￯　-〿]/
/** 中日韩逐字断行，其他文字按单词断行，超长单词再逐字拆开。 */
function tokens(text: string): string[] {
  return text.replace(/\s+/g, ' ').trim().match(new RegExp(`${cjk.source}|[^\\s\\u2e80-\\u9fff\\uac00-\\ud7af\\uff00-\\uffef\\u3000-\\u303f]+\\s?|\\s`, 'g')) ?? []
}

function wrap(text: string, maxWidth: number, measure: (value: string) => number): string[] {
  const lines: string[] = []
  let current = ''
  for (const token of tokens(text)) {
    const next = current + token
    if (!current || measure(next.trimEnd()) <= maxWidth) {
      if (current || measure(token.trimEnd()) <= maxWidth) { current = next; continue }
    } else {
      lines.push(current.trimEnd())
      current = ''
      if (measure(token.trimEnd()) <= maxWidth) { current = token.trimStart(); continue }
    }
    // 单个单词比框还宽：逐字放入，避免整段溢出。
    for (const char of token) {
      if (current && measure((current + char).trimEnd()) > maxWidth) { lines.push(current.trimEnd()); current = '' }
      current += char
    }
  }
  if (current.trim()) lines.push(current.trimEnd())
  return lines
}

export const LINE_SPACING = 1.2
/**
 * 译文放进原文所在的框：从原文行高起步，逐步缩小字号直到全部行放得下；
 * 最小字号仍放不下时保留最小字号（允许少量溢出），不把文字缩到无法阅读。
 * measure(text, fontSize) 返回该字号下的文字宽度，由调用方用 canvas 测量。
 */
export function fitText(text: string, box: Pick<OverlayBox, 'width' | 'height'>, lineHeight: number, measure: (value: string, fontSize: number) => number, minFont = 8): OverlayLayout {
  let size = Math.max(minFont, Math.min(lineHeight * 0.85, box.height / LINE_SPACING))
  for (;;) {
    const lines = wrap(text, box.width, value => measure(value, size))
    if (lines.length * size * LINE_SPACING <= box.height * 1.05 || size <= minFont) return { fontSize: size, lines }
    size = Math.max(minFont, size * 0.92)
  }
}
