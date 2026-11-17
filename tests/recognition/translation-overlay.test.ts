import { describe, expect, it } from 'vitest'
import type { OcrLine } from '../../src/features/recognition/ai'
import { fitText, matchesSegments, overlayBoxes, overlayParagraphs, sampleColors } from '../../src/features/recognition/translation-overlay'

function line(order: number, text: string, x: number, y: number, width: number, height: number, skew = 0): OcrLine {
  return { id: `line-${order}`, text, confidence: 0.9, language: null, order, paragraphId: null,
    polygon: [{ x, y }, { x: x + width, y: y + skew }, { x: x + width, y: y + height + skew }, { x, y: y + height }] }
}

describe('译文覆盖：段落合并', () => {
  it('上下相邻且行高相近的行合并为一段，英文断词拼接', () => {
    const result = overlayParagraphs([
      line(0, 'The quick brown fox jumps over the lazy', 10, 10, 300, 20),
      line(1, 'dog and keeps run-', 10, 34, 200, 20),
      line(2, 'ning.', 10, 58, 60, 20),
      line(3, 'Second paragraph', 10, 140, 180, 20),
    ])!
    expect(result.map(item => item.text)).toEqual(['The quick brown fox jumps over the lazy dog and keeps running.', 'Second paragraph'])
    expect(result[0]).toMatchObject({ ids: ['line-0', 'line-1', 'line-2'], box: { x: 10, y: 10, width: 300, height: 68 }, lineHeight: 20 })
  })
  it('同一行左右两列、字号差异大的标题各自成段', () => {
    const result = overlayParagraphs([
      line(0, 'Title', 10, 0, 200, 48),
      line(1, 'left column', 10, 60, 150, 18),
      line(2, 'right column', 400, 60, 150, 18),
    ])!
    expect(result.map(item => item.text)).toEqual(['Title', 'left column', 'right column'])
  })
  it('中文行之间不加空格', () => {
    expect(overlayParagraphs([line(0, '截图翻译', 0, 0, 80, 20), line(1, '覆盖原文', 0, 24, 80, 20)])![0].text).toBe('截图翻译覆盖原文')
  })
  it('缺少坐标、明显倾斜、没有文字或段落过多时不覆盖', () => {
    expect(overlayParagraphs([{ ...line(0, 'a', 0, 0, 10, 10), polygon: null }])).toBeNull()
    expect(overlayParagraphs([line(0, 'tilted', 0, 0, 100, 20, 40)])).toBeNull()
    expect(overlayParagraphs([line(0, '   ', 0, 0, 10, 10)])).toBeNull()
    expect(overlayParagraphs(Array.from({ length: 51 }, (_, i) => line(i, `p${i}`, 0, i * 100, 50, 20)))).toBeNull()
  })
})

describe('译文覆盖：原文核对与坐标', () => {
  it('忽略空白差异判断原文未改', () => {
    expect(matchesSegments('Hello\nworld\n\nNext', ['Hello world', 'Next'])).toBe(true)
    expect(matchesSegments('Hello there', ['Hello world'])).toBe(false)
    // 合并行去掉了行尾断词连字符，不能因此退回普通翻译。
    expect(matchesSegments('Advocacy team -\nincluding', ['Advocacy team including'])).toBe(true)
    expect(matchesSegments('anything', null)).toBe(false)
    expect(matchesSegments('', [])).toBe(false)
  })
  it('按截图选区偏移映射，宽高比不一致时放弃', () => {
    const paragraph = { ids: ['a'], text: 'a', box: { x: 10, y: 20, width: 30, height: 40 }, lineHeight: 20 }
    const image = { url: 'blob:x', width: 1000, height: 800, crop: { x: 100, y: 50, width: 200, height: 100 } }
    expect(overlayBoxes(image, { imageWidth: 200, imageHeight: 100 }, [paragraph])).toEqual([{ x: 110, y: 70, width: 30, height: 40 }])
    expect(overlayBoxes(image, { imageWidth: 100, imageHeight: 100 }, [paragraph])).toBeNull()
  })
})

function canvas(width: number, height: number, background: number[], ink: number[], inkRect: { x: number; y: number; w: number; h: number }) {
  const data = new Uint8ClampedArray(width * height * 4)
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const inside = x >= inkRect.x && x < inkRect.x + inkRect.w && y >= inkRect.y && y < inkRect.y + inkRect.h && (x + y) % 3 === 0
    data.set([...(inside ? ink : background), 255], (y * width + x) * 4)
  }
  return { data, width, height }
}

describe('译文覆盖：取色', () => {
  it('背景取外沿众数，文字取框内差异最大的像素', () => {
    const pixels = canvas(60, 40, [250, 240, 230], [20, 30, 120], { x: 10, y: 10, w: 40, h: 20 })
    expect(sampleColors(pixels, { x: 10, y: 10, width: 40, height: 20 })).toEqual({ background: 'rgb(250, 240, 230)', foreground: 'rgb(20, 30, 120)' })
  })
  it('框内没有对比时按背景亮度给出黑或白', () => {
    expect(sampleColors(canvas(20, 20, [30, 30, 30], [30, 30, 30], { x: 0, y: 0, w: 0, h: 0 }), { x: 5, y: 5, width: 10, height: 10 }).foreground).toBe('rgb(245, 245, 245)')
    expect(sampleColors(canvas(20, 20, [240, 240, 240], [240, 240, 240], { x: 0, y: 0, w: 0, h: 0 }), { x: 5, y: 5, width: 10, height: 10 }).foreground).toBe('rgb(17, 17, 17)')
  })
  it('框贴边时不越界读取', () => {
    expect(() => sampleColors(canvas(10, 10, [0, 0, 0], [255, 255, 255], { x: 0, y: 0, w: 10, h: 10 }), { x: 0, y: 0, width: 10, height: 10 })).not.toThrow()
  })
})

describe('译文覆盖：排版', () => {
  const measure = (value: string, size: number) => value.length * size * 0.5
  it('放得下时沿用原文行高并按单词换行', () => {
    const layout = fitText('hello world foo', { width: 60, height: 60 }, 20, measure)
    expect(layout.fontSize).toBe(17)
    expect(layout.lines).toEqual(['hello', 'world', 'foo'])
  })
  it('放不下时缩小字号，直到所有行都在框内', () => {
    const layout = fitText('a much longer translated sentence than the original', { width: 100, height: 40 }, 20, measure)
    expect(layout.fontSize).toBeLessThan(17)
    expect(layout.lines.length * layout.fontSize * 1.2).toBeLessThanOrEqual(42)
    expect(layout.lines.join(' ')).toBe('a much longer translated sentence than the original')
  })
  it('中文逐字换行，超长单词逐字拆开，最小字号不再缩小', () => {
    expect(fitText('截图翻译覆盖原文', { width: 40, height: 100 }, 20, measure).lines.every(item => item.length <= 5)).toBe(true)
    const tight = fitText('supercalifragilistic', { width: 20, height: 5 }, 20, measure, 8)
    expect(tight.fontSize).toBe(8)
    expect(tight.lines.join('')).toBe('supercalifragilistic')
  })
})
