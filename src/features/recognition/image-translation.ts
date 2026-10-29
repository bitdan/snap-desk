import type { AiSettings, OcrLine, OcrResult, TranslationResult, TranslationRequest, TranslationBatchResult } from '@/features/recognition/ai'
import type { Rect } from '@/shared/geometry/selection'

export interface TranslationImage { dataUrl: string; width: number; height: number; crop?: Rect }
export interface ImageTextUnit { id: string; source: string; bounds: Rect | null }
export interface ImageTranslationPlan { units: ImageTextUnit[]; reason: string; error: string }

/** 仅水平、完整、可信的四边形允许贴回原图；不靠译文换行推测框的归属。 */
function lineBounds(line: OcrLine, width: number, height: number): Rect | null {
  const points = line.polygon
  if (!points || points.length !== 4 || (line.confidence !== null && (!Number.isFinite(line.confidence) || line.confidence < 0.7))) return null
  if (points.some(p => !Number.isFinite(p.x) || !Number.isFinite(p.y) || p.x < 0 || p.y < 0 || p.x > width || p.y > height)) return null
  const x = Math.min(...points.map(p => p.x)), y = Math.min(...points.map(p => p.y))
  const w = Math.max(...points.map(p => p.x)) - x, h = Math.max(...points.map(p => p.y)) - y
  if (w < 2 || h < 2) return null
  // 每个点应接近矩形四角；旋转文字与透视较强的文字交给对照视图。
  const tolerance = Math.max(1, Math.min(w, h) * 0.12)
  const corners = new Set(points.map(p => `${Math.abs(p.x - x) <= tolerance ? 0 : Math.abs(p.x - x - w) <= tolerance ? 1 : 'x'},${Math.abs(p.y - y) <= tolerance ? 0 : Math.abs(p.y - y - h) <= tolerance ? 1 : 'y'}`))
  if (corners.size !== 4 || [...corners].some(corner => /[xy]/.test(corner))) return null
  return { x, y, width: w, height: h }
}

/** 复杂布局整段对照：每组最多 32 行，兼容本地 Worker 的 64 行请求上限。 */
function comparisonUnits(text: string): ImageTextUnit[] {
  const lines = text.split('\n'), units: ImageTextUnit[] = []
  for (let index = 0; index < lines.length; index += 32) {
    const source = lines.slice(index, index + 32).join('\n')
    if (source.trim()) units.push({ id: `text-${index}`, source, bounds: null })
  }
  return units
}

export function imageTranslationPlan(result: OcrResult, source = result.fullText): ImageTranslationPlan {
  const comparison = (reason: string, error = '') => ({ units: comparisonUnits(source), reason, error })
  if (new TextEncoder().encode(source).length > 32_000) return comparison('文字较多，显示文本对照。', '识别文字超过 32 KB，请选择较小的图片区域。')
  if (!source.trim()) return comparison('未识别到可翻译的文字。')
  if (source !== result.fullText) return comparison('原文已修改，显示文本对照以保留原始定位。')
  const lines = [...result.lines].sort((a, b) => a.order - b.order)
  if (!lines.length || lines.length > 64 || lines.map(line => line.text).join('\n') !== source) return comparison('文字密集或段落对应不明确，显示文本对照。')
  const ids = new Set<string>(), units: ImageTextUnit[] = []
  if (!Number.isFinite(result.imageWidth) || !Number.isFinite(result.imageHeight) || result.imageWidth <= 0 || result.imageHeight <= 0) return comparison('图片尺寸无效，显示文本对照。')
  for (const line of lines) {
    const bounds = lineBounds(line, result.imageWidth, result.imageHeight)
    if (!bounds || !line.id || ids.has(line.id) || !line.text.trim()) return comparison('部分文字缺少可靠位置，显示文本对照。')
    ids.add(line.id)
    for (const existing of units) {
      const other = existing.bounds!
      const overlap = Math.max(0, Math.min(bounds.x + bounds.width, other.x + other.width) - Math.max(bounds.x, other.x))
        * Math.max(0, Math.min(bounds.y + bounds.height, other.y + other.height) - Math.max(bounds.y, other.y))
      if (overlap > Math.min(bounds.width * bounds.height, other.width * other.height) * 0.15) return comparison('文字框重叠，显示文本对照。')
    }
    units.push({ id: line.id, source: line.text, bounds })
  }
  return { units, reason: '', error: '' }
}

/** 覆盖层按图片比例移动，不随窗口 DPI 改变；限制到原图范围内。 */
export function overlayPosition(x: number, y: number): { x: number; y: number } {
  return { x: Math.max(0, Math.min(0.9, x)), y: Math.max(0, Math.min(0.9, y)) }
}

/** 按实际显示尺寸保守检查译文卡片；放不下时回到对照视图，避免遮盖成一团。 */
export function overlayLayoutReason(plan: ImageTranslationPlan, imageWidth: number, imageHeight: number, displayWidth: number, displayHeight: number, results: Record<string, string>): string {
  if (!displayWidth || !displayHeight) return ''
  const placed: Rect[] = []
  for (const unit of plan.units) {
    if (!unit.bounds || !results[unit.id]) continue
    const p = overlayPosition(unit.bounds.x / imageWidth, unit.bounds.y / imageHeight)
    const width = Math.min(0.8, Math.max(0.2, unit.bounds.width / imageWidth), 1 - p.x) * displayWidth
    const charsPerLine = Math.max(1, Math.floor((width - 8) / 13))
    const rows = Math.ceil(unit.source.length / charsPerLine) + Math.ceil(results[unit.id]!.length / charsPerLine)
    const height = Math.min(200, 40 + rows * 20)
    const rect = { x: p.x * displayWidth, y: p.y * displayHeight, width, height }
    if (width < 120 || rect.y + height > displayHeight || placed.some(other => rect.x < other.x + other.width && rect.x + rect.width > other.x && rect.y < other.y + other.height && rect.y + rect.height > other.y)) {
      return '译文在当前窗口中无法清晰排布，显示文本对照。'
    }
    placed.push(rect)
  }
  return ''
}

export interface ImageTranslationTransport {
  translate(id: string, text: string, consent: boolean, settings: AiSettings): Promise<TranslationResult>
  translateBatch?(id: string, request: TranslationRequest, consent: boolean, settings: AiSettings): Promise<TranslationBatchResult>
  newId(): string
  cancelled(): boolean
  pending(id: string | null): void
  completed(unit: ImageTextUnit, result: TranslationResult): void
}

/** 在线按最多 50 个 OCR 单元批量请求；本地保持逐段调用，取消后不提交迟到结果。 */
export async function translateImageUnits(units: ImageTextUnit[], settings: AiSettings, consent: boolean, transport: ImageTranslationTransport): Promise<void> {
  if (settings.provider !== 'local-ctranslate2' && !consent) throw new Error('请先确认发送识别文本。')
  if (settings.provider !== 'local-ctranslate2' && transport.translateBatch) {
    for (let offset = 0; offset < units.length; offset += 50) {
      if (transport.cancelled()) throw new Error('翻译已取消。')
      const batch = units.slice(offset, offset + 50), id = transport.newId()
      transport.pending(id)
      try {
        const result = await transport.translateBatch(id, { texts: batch.map(unit => unit.source), sourceLanguage: null, targetLanguage: settings.targetLanguage }, consent, settings)
        if (transport.cancelled()) throw new Error('翻译已取消。')
        if (result.requestId !== id || result.targetLanguage !== settings.targetLanguage || result.translations.length !== batch.length
          || result.translations.some((item, index) => item.sourceText !== batch[index]?.source || !item.translatedText.trim())) throw new Error('翻译结果与当前请求不匹配或为空，请重试。')
        batch.forEach((unit, index) => transport.completed(unit, { requestId: id, targetLanguage: result.targetLanguage, translatedText: result.translations[index]!.translatedText }))
      } finally { transport.pending(null) }
    }
    return
  }
  for (const unit of units) {
    if (transport.cancelled()) throw new Error('翻译已取消。')
    const id = transport.newId()
    transport.pending(id)
    try {
      const result = await transport.translate(id, unit.source, consent, settings)
      if (transport.cancelled()) throw new Error('翻译已取消。')
      if (result.requestId !== id || result.targetLanguage !== settings.targetLanguage || !result.translatedText.trim()) throw new Error('翻译结果与当前请求不匹配或为空，请重试。')
      transport.completed(unit, result)
    } finally { transport.pending(null) }
  }
}
