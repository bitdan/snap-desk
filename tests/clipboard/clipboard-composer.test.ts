import { afterEach, expect, it, vi } from 'vitest'
import {
  createEntries, exportPayload, invalidEntry, loadComposerOptions, longPngSize, moveEntry, pdfPages, separateNames, wrapText,
  defaultComposerOptions, type ComposerEntry,
} from '../../src/features/clipboard/clipboard-composer'
import type { ClipboardItem } from '../../src/features/clipboard/clipboard'

const item = (id: number, kind: ClipboardItem['kind'], extra: Partial<ClipboardItem> = {}) => ({ id, kind, preview: `预览 ${id}`, width: null, height: null, ...extra }) as ClipboardItem
const text = (id: number, original: string, edited?: string): ComposerEntry => ({ id, kind: 'text', width: 1, height: 1, original, preview: '', ...(edited === undefined ? {} : { text: edited }) })
const image = (id: number, width: number, height: number): ComposerEntry => ({ id, kind: 'image', width, height, original: null, preview: '' })

afterEach(() => vi.unstubAllGlobals())

it('素材快照保持选择顺序并排除文件引用；移动只调整位置', () => {
  const entries = createEntries([item(3, 'image', { width: 800, height: 600 }), item(1, 'text'), item(4, 'file')])
  expect(entries.map(entry => [entry.id, entry.kind])).toEqual([[3, 'image'], [1, 'text']])
  expect(moveEntry([1, 2, 3], 0, 2)).toEqual([2, 3, 1])
  expect(moveEntry([1, 2, 3], 0, 5)).toEqual([1, 2, 3])
})

it('只有真正改写过的文本随导出提交；清空改写被识别为无效', () => {
  const options = defaultComposerOptions()
  expect(exportPayload([text(1, '原文'), image(2, 10, 10)], options)).toEqual({ ...options, ids: [1, 2] })
  expect(exportPayload([text(1, '原文', '原文'), text(2, '旧', '新')], options).texts).toEqual([null, '新'])
  expect(invalidEntry([text(1, '原文', '  ')])).toBe(0)
  expect(invalidEntry([text(1, '原文', 'x')])).toBe(-1)
})

it('PDF 预览与导出规则一致：长文本按 A4 分页，图片每张一页并支持原始比例', () => {
  const options = { ...defaultComposerOptions(), marginMm: 10 }
  const long = text(1, Array.from({ length: 120 }, (_, i) => `第${i}行`).join('\n'))
  const pages = pdfPages([long, image(2, 960, 480)], options)
  expect(pages.filter(page => page.kind === 'text')).toHaveLength(3)
  expect(pages.at(-1)).toMatchObject({ kind: 'image', width: 210, height: 297, entry: 1 })
  const original = pdfPages([image(2, 960, 480)], { ...options, pageSize: 'original' })[0]
  expect(original.width).toBeCloseTo(960 * 25.4 / 96 + 20)
  expect(wrapText('中文中文', 2)).toEqual(['中文', '中文'])
})

it('长图尺寸按最宽素材（上限 2400）缩放并累加间距；分别导出按序命名', () => {
  const size = longPngSize([image(1, 1000, 500), image(2, 500, 500)], { ...defaultComposerOptions(), gapPx: 20 })
  expect(size).toEqual({ width: 1000, height: 500 + 1000 + 20 })
  expect(longPngSize([image(1, 4800, 100)], defaultComposerOptions()).width).toBe(2400)
  expect(separateNames([text(1, 'a'), image(2, 1, 1)])).toEqual(['001.txt', '002.png'])
})

it('导出参数偏好损坏或越界时回到安全值', () => {
  vi.stubGlobal('localStorage', { getItem: () => JSON.stringify({ mode: 'longPng', gapPx: 999, marginMm: 'x', background: 'transparent' }) })
  expect(loadComposerOptions()).toEqual({ mode: 'longPng', pageSize: 'A4', gapPx: 200, marginMm: 12, background: 'transparent' })
  vi.stubGlobal('localStorage', { getItem: () => '{' })
  expect(loadComposerOptions()).toEqual(defaultComposerOptions())
})
