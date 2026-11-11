/** 组合编辑器的纯逻辑：素材顺序、改写、导出参数与近似排版，参数含义与 Rust export.rs 一致。 */
import type { ClipboardExportOptions, ClipboardItem } from '@/features/clipboard/clipboard'

export type ComposerMode = ClipboardExportOptions['mode']
export type ComposerOptions = Omit<ClipboardExportOptions, 'ids' | 'texts'>
export interface ComposerEntry {
  id: number; kind: 'text' | 'image'; width: number; height: number
  /** 历史原文；读取完成前为 null，此时不能改写。 */
  original: string | null
  /** 改写后的文本；undefined 表示沿用原文。 */
  text?: string
  preview: string
}

export const defaultComposerOptions = (): ComposerOptions => ({ mode: 'pdf', pageSize: 'A4', gapPx: 16, marginMm: 12, background: 'white' })
const optionsKey = 'snap-desk.clipboard-compose.v1'

/** 导出参数只作为本机偏好记住；存储不可用或内容损坏时回到默认值。 */
export function loadComposerOptions(): ComposerOptions {
  const fallback = defaultComposerOptions()
  try {
    const value = JSON.parse(localStorage.getItem(optionsKey) ?? 'null') as Partial<ComposerOptions> | null
    if (!value) return fallback
    return {
      mode: ['pdf', 'longPng', 'separate'].includes(value.mode as string) ? value.mode! : fallback.mode,
      pageSize: value.pageSize === 'original' ? 'original' : 'A4',
      gapPx: clamp(value.gapPx, 0, 200, fallback.gapPx),
      marginMm: clamp(value.marginMm, 0, 30, fallback.marginMm),
      background: value.background === 'transparent' ? 'transparent' : 'white',
    }
  } catch { return fallback }
}
export function saveComposerOptions(options: ComposerOptions) {
  try { localStorage.setItem(optionsKey, JSON.stringify(options)) } catch { /* 偏好写不进去不影响本次导出。 */ }
}
export function clamp(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback
}

export function createEntries(items: ClipboardItem[]): ComposerEntry[] {
  return items.filter(item => item.kind !== 'file').map(item => ({
    id: item.id, kind: item.kind as 'text' | 'image', width: item.width ?? 1, height: item.height ?? 1,
    original: null, preview: item.preview ?? '',
  }))
}
export function entryText(entry: ComposerEntry): string {
  return entry.text ?? entry.original ?? entry.preview
}
export function edited(entry: ComposerEntry): boolean {
  return entry.kind === 'text' && entry.text !== undefined && entry.original !== null && entry.text !== entry.original
}
export function moveEntry<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return list
  const next = [...list]
  const [item] = next.splice(from, 1)
  next.splice(to, 0, item)
  return next
}
/** 只在有改写时携带 texts，后端据此按位置替换文本，不回写历史。 */
export function exportPayload(entries: ComposerEntry[], options: ComposerOptions): ClipboardExportOptions {
  const texts = entries.map(entry => edited(entry) ? entry.text! : null)
  return { ...options, ids: entries.map(entry => entry.id), ...(texts.some(text => text !== null) ? { texts } : {}) }
}
export function invalidEntry(entries: ComposerEntry[]): number {
  return entries.findIndex(entry => entry.kind === 'text' && entry.text !== undefined && !entry.text.trim())
}

/** 按字符近似字宽：CJK 记 1 em，其余 0.55 em；只用于预览分页，最终排版以导出字体为准。 */
function charEm(ch: string): number {
  const code = ch.codePointAt(0) ?? 0
  return code >= 0x2e80 ? 1 : code === 9 ? 2 : 0.55
}
export function wrapText(text: string, maxEm: number): string[] {
  const lines: string[] = []
  for (const paragraph of text.replace(/\r\n?/g, '\n').split('\n')) {
    let line = '', width = 0
    for (const ch of paragraph) {
      const em = charEm(ch)
      if (width + em > maxEm && line) { lines.push(line); line = ''; width = 0 }
      line += ch; width += em
    }
    lines.push(line)
  }
  return lines
}

export const PDF_FONT_MM = 11 * 25.4 / 72
export const PDF_LINE_MM = 5.5
export type PdfPage =
  | { kind: 'text'; entry: number; width: number; height: number; lines: string[] }
  | { kind: 'image'; entry: number; width: number; height: number }
/** 与 export.rs 一致：文本按 A4 分页，图片每张一页，原始比例页面按 96 DPI 换算。 */
export function pdfPages(entries: ComposerEntry[], options: ComposerOptions): PdfPage[] {
  const margin = options.marginMm, pages: PdfPage[] = []
  entries.forEach((entry, index) => {
    if (entry.kind === 'text') {
      const lines = wrapText(entryText(entry), (210 - 2 * margin) / PDF_FONT_MM)
      const perPage = Math.max(1, Math.floor((297 - 2 * margin - 6) / PDF_LINE_MM))
      for (let start = 0; start < lines.length; start += perPage) pages.push({ kind: 'text', entry: index, width: 210, height: 297, lines: lines.slice(start, start + perPage) })
      return
    }
    const mm = (px: number) => px * 25.4 / 96
    pages.push(options.pageSize === 'original'
      ? { kind: 'image', entry: index, width: mm(entry.width) + margin * 2, height: mm(entry.height) + margin * 2 }
      : { kind: 'image', entry: index, width: 210, height: 297 })
  })
  return pages
}

/** 长图文本块：宽 1200px、内边距 24px、行高 34px，字号约 16.6px，与 text_image 渲染一致。 */
export const LONG_TEXT_WIDTH = 1200
export function longPngSize(entries: ComposerEntry[], options: ComposerOptions): { width: number; height: number } {
  if (!entries.length) return { width: 0, height: 0 }
  const dims = entries.map(entry => entry.kind === 'image'
    ? [entry.width, entry.height]
    : [LONG_TEXT_WIDTH, 48 + wrapText(entryText(entry), 1152 / 16.6).length * 34])
  const width = Math.min(2400, Math.max(...dims.map(([w]) => w)))
  const height = dims.reduce((sum, [w, h]) => sum + Math.max(1, Math.floor(h * width / Math.max(1, w))), 0) + options.gapPx * (entries.length - 1)
  return { width, height }
}
export const LONG_PNG_MAX_PIXELS = 32_000_000
export const PDF_MAX_PIXELS = 64_000_000
export const PDF_MAX_PAGES = 1000

export function separateNames(entries: ComposerEntry[]): string[] {
  return entries.map((entry, index) => `${String(index + 1).padStart(3, '0')}.${entry.kind === 'text' ? 'txt' : 'png'}`)
}
