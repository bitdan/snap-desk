import { expect, it } from 'vitest'
import { lineRanges, mergeLines, ocrDisplaySize, ocrImageView, ocrWindowSize, tidySpaces } from '../../src/features/recognition/ocr-workspace'
import type { OcrLine, OcrResult } from '../../src/features/recognition/ai'

const line = (id: string, text: string, order: number, polygon: OcrLine['polygon'] = null): OcrLine => ({ id, text, order, polygon, confidence: null, language: null, paragraphId: null })
const result = (width: number, height: number): OcrResult => ({ requestId: 'r', provider: 'test', imageWidth: width, imageHeight: height, fullText: '', lines: [], elapsedMs: 0, warnings: [] })

it('合并换行：英文用空格、中文直接相连、断词连字符去掉，空行保留为段落', () => {
  expect(mergeLines('hello\nworld')).toBe('hello world')
  expect(mergeLines('第一行\n第二行')).toBe('第一行第二行')
  expect(mergeLines('inter-\nnational')).toBe('international')
  expect(mergeLines('a\nb\n\nc')).toBe('a b\n\nc')
})

it('整理空格：去掉汉字间空格与行首尾空白，压缩连续空格', () => {
  expect(tidySpaces('  中 文 识 别  ')).toBe('中文识别')
  expect(tidySpaces('hello    world\n  next ')).toBe('hello world\nnext')
})

it('按阅读顺序定位每行，被改写的行跳过且不影响后续行', () => {
  const lines = [line('a', 'foo', 0), line('b', 'changed', 1), line('c', 'foo', 2)]
  expect(lineRanges('foo\nbar\nfoo', lines)).toEqual([{ id: 'a', start: 0, end: 3 }, { id: 'c', start: 8, end: 11 }])
})

it('识别框按选区偏移和缩放映射；宽高比不一致时不画框', () => {
  const lines = [line('a', 'x', 0, [{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 50 }])]
  const view = ocrImageView({ url: 'u', width: 1000, height: 800, crop: { x: 10, y: 20, width: 200, height: 100 } }, result(100, 50), lines)
  expect(view.viewBox).toBe('10 20 200 100')
  expect(view.boxes[0].points).toBe('10,20 210,20 210,120')
  expect(ocrImageView({ url: 'u', width: 100, height: 200 }, result(200, 100), lines).boxes).toEqual([])
})

it('原图按截图屏幕尺寸显示，窗口随原图大小并限制在屏幕内', () => {
  const image = { url: 'u', width: 3000, height: 2000, crop: { x: 0, y: 0, width: 600, height: 300 } }
  expect(ocrDisplaySize(image, 1.5)).toEqual({ width: 400, height: 200 })
  expect(ocrWindowSize(image, 1.5, { width: 1920, height: 1080 })).toEqual({ width: 824, height: 620 })
  const huge = { url: 'u', width: 4000, height: 3000 }
  expect(ocrWindowSize(huge, 1, { width: 1920, height: 1080 })).toEqual({ width: 1766, height: 950 })
})
