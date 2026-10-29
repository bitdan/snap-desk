import { describe, expect, it, vi } from 'vitest'
import { imageTranslationPlan, overlayLayoutReason, overlayPosition, translateImageUnits, type ImageTranslationTransport } from '../../src/features/recognition/image-translation'
import { defaultAiSettings, type OcrLine, type OcrResult } from '../../src/features/recognition/ai'

function line(id: string, order: number, y = order * 150): OcrLine {
  return { id, order, text: `text-${id}`, confidence: 0.95, language: null, paragraphId: null,
    polygon: [{ x: 10, y }, { x: 310, y }, { x: 310, y: y + 20 }, { x: 10, y: y + 20 }] }
}
function recognition(lines = [line('a', 0), line('b', 1)]): OcrResult {
  return { requestId: 'ocr-test', provider: 'local-ppocr', imageWidth: 1000, imageHeight: 1000, elapsedMs: 1, warnings: [],
    lines, fullText: [...lines].sort((a, b) => a.order - b.order).map(line => line.text).join('\n') }
}
function transport(overrides: Partial<ImageTranslationTransport> = {}): ImageTranslationTransport {
  let sequence = 0
  return { newId: () => `request-${++sequence}`, cancelled: () => false, pending: vi.fn(), completed: vi.fn(),
    translate: vi.fn(async (requestId, text, _consent, settings) => ({ requestId, translatedText: `${text} translated`, targetLanguage: settings.targetLanguage })), ...overrides }
}

describe('图片翻译定位与降级', () => {
  it('按 OCR 阅读顺序生成稳定行 ID 与原图像素位置，不改变原结果', () => {
    const input = recognition([line('b', 1), line('a', 0)])
    const plan = imageTranslationPlan(input)
    expect(plan.reason).toBe('')
    expect(plan.units.map(unit => unit.id)).toEqual(['a', 'b'])
    expect(plan.units[0]?.bounds).toEqual({ x: 10, y: 0, width: 300, height: 20 })
    expect(input.lines[0]?.id).toBe('b')
  })
  it.each(['missing', 'outside', 'nan', 'rotated', 'confidence', 'repeated-point'])('位置不可靠时整图降级：%s', kind => {
    const a = line('a', 0)
    if (kind === 'missing') a.polygon = null
    if (kind === 'outside') a.polygon![0]!.x = -1
    if (kind === 'nan') a.polygon![0]!.x = NaN
    if (kind === 'rotated') a.polygon![1]!.y = 10
    if (kind === 'confidence') a.confidence = 0.6
    if (kind === 'repeated-point') a.polygon![3] = a.polygon![0]!
    const plan = imageTranslationPlan(recognition([a]))
    expect(plan.reason).not.toBe('')
    expect(plan.units[0]?.bounds).toBeNull()
  })
  it('重叠文字框、重复 ID 不误贴回原图', () => {
    expect(imageTranslationPlan(recognition([line('a', 0), line('b', 1, 2)])).reason).not.toBe('')
    expect(imageTranslationPlan(recognition([line('a', 0), line('a', 1)])).reason).not.toBe('')
  })
  it('修改原文后保留 OCR，译文只显示对照', () => {
    const input = recognition()
    const plan = imageTranslationPlan(input, '修正后的原文')
    expect(plan.units[0]).toEqual({ id: 'text-0', source: '修正后的原文', bounds: null })
    expect(input.fullText).toBe('text-a\ntext-b')
  })
  it('密集版式按 32 行分组，兼容本地行数上限，不按译文换行猜位置', () => {
    const input = recognition(Array.from({ length: 70 }, (_, index) => line(String(index), index)))
    const plan = imageTranslationPlan(input)
    expect(plan.reason).not.toBe('')
    expect(plan.units.map(unit => unit.source.split('\n').length)).toEqual([32, 32, 6])
    expect(plan.units.map(unit => unit.source).join('\n')).toBe(input.fullText)
  })
  it('空结果不生成请求，按 UTF-8 字节限制文本', () => {
    expect(imageTranslationPlan(recognition([])).units).toEqual([])
    expect(imageTranslationPlan(recognition(), '中'.repeat(11000)).error).toContain('32 KB')
  })
  it('原图尺寸无效或文本与框不对应时显示对照', () => {
    const input = recognition()
    expect(imageTranslationPlan({ ...input, imageWidth: 0 }).reason).not.toBe('')
    expect(imageTranslationPlan({ ...input, fullText: 'unmapped text' }).units[0]?.bounds).toBeNull()
  })
  it('移动位置约束在图像内，和 DPI 无关', () => {
    expect(overlayPosition(-1, 2)).toEqual({ x: 0, y: 0.9 })
    expect(overlayPosition(0.25, 0.75)).toEqual({ x: 0.25, y: 0.75 })
  })
  it('实际显示过窄、卡片重叠或长译文溢出时降级，充足空间允许覆盖层', () => {
    const plan = imageTranslationPlan(recognition())
    expect(overlayLayoutReason(plan, 1000, 1000, 1000, 1000, { a: '译文', b: '译文' })).toBe('')
    expect(overlayLayoutReason(plan, 1000, 1000, 200, 200, { a: '译文' })).not.toBe('')
    expect(overlayLayoutReason(plan, 1000, 1000, 1000, 1000, { a: '长'.repeat(1000), b: '译文' })).not.toBe('')
    expect(overlayLayoutReason(plan, 1000, 1000, 0, 0, { a: '译文' })).toBe('')
  })
})

describe('图片翻译请求与取消', () => {
  const online = { ...defaultAiSettings(), provider: 'deepl' as const }
  function batched(overrides: Partial<ImageTranslationTransport> = {}) {
    return transport({ translateBatch: vi.fn(async (requestId, request) => ({ requestId, targetLanguage: request.targetLanguage,
      translations: request.texts.map((sourceText: string) => ({ sourceText, translatedText: `${sourceText}\ntranslated` })),
    })), ...overrides })
  }
  it('在线批量发送文字并保留 OCR 归属；50 个单元分界后继续下一批', async () => {
    const units = Array.from({ length: 51 }, (_, i) => ({ id: `unit-${i}`, source: `source-${i}`, bounds: null }))
    const io = batched()
    await translateImageUnits(units, online, true, io)
    expect(io.translate).not.toHaveBeenCalled()
    expect(io.translateBatch).toHaveBeenCalledTimes(2)
    expect(io.translateBatch).toHaveBeenNthCalledWith(1, 'request-1', { texts: units.slice(0, 50).map(unit => unit.source), sourceLanguage: null, targetLanguage: 'zh-CN' }, true, online)
    expect(io.completed).toHaveBeenCalledTimes(51)
    expect(io.completed).toHaveBeenNthCalledWith(51, units[50], expect.objectContaining({ translatedText: 'source-50\ntranslated' }))
  })
  it('DeepL 未确认不上传；取消后丢弃整批迟到结果', async () => {
    const units = imageTranslationPlan(recognition()).units, io = batched()
    await expect(translateImageUnits(units, online, false, io)).rejects.toThrow('确认')
    expect(io.translateBatch).not.toHaveBeenCalled()
    let cancelled = false
    const late = batched({ cancelled: () => cancelled, translateBatch: vi.fn(async (requestId, request) => {
      cancelled = true
      return { requestId, targetLanguage: request.targetLanguage, translations: request.texts.map((sourceText: string) => ({ sourceText, translatedText: 'late' })) }
    }) })
    await expect(translateImageUnits(units, online, true, late)).rejects.toThrow('取消')
    expect(late.completed).not.toHaveBeenCalled()
    expect(late.pending).toHaveBeenLastCalledWith(null)
  })
  it.each(['order', 'count', 'empty', 'id', 'target'])('批量响应异常时整批拒绝：%s', invalid => {
    const io = batched({ translateBatch: vi.fn(async (requestId, request) => {
      const translations = request.texts.map((sourceText: string) => ({ sourceText, translatedText: 'translated' }))
      if (invalid === 'order') translations.reverse()
      if (invalid === 'count') translations.pop()
      if (invalid === 'empty') translations[1]!.translatedText = ' '
      return { requestId: invalid === 'id' ? 'stale' : requestId, targetLanguage: invalid === 'target' ? 'fr' : request.targetLanguage, translations }
    }) })
    return expect(translateImageUnits(imageTranslationPlan(recognition()).units, online, true, io)).rejects.toThrow('不匹配或为空').then(() => {
      expect(io.completed).not.toHaveBeenCalled()
      expect(io.pending).toHaveBeenLastCalledWith(null)
    })
  })
  it('逐行翻译并按请求归属提交；模型多行输出仍属于原来的一行', async () => {
    const io = transport({ translate: vi.fn(async (requestId, _text, consent, settings) => {
      expect(consent).toBe(false)
      return { requestId, translatedText: 'first\nsecond', targetLanguage: settings.targetLanguage }
    }) })
    const plan = imageTranslationPlan(recognition())
    await translateImageUnits(plan.units, defaultAiSettings(), false, io)
    expect(io.translate).toHaveBeenCalledTimes(2)
    expect(io.completed).toHaveBeenNthCalledWith(1, plan.units[0], expect.objectContaining({ requestId: 'request-1', translatedText: 'first\nsecond' }))
    expect(io.completed).toHaveBeenNthCalledWith(2, plan.units[1], expect.objectContaining({ requestId: 'request-2' }))
    expect(io.pending).toHaveBeenLastCalledWith(null)
  })
  it('在线未确认时不发送任何原文', async () => {
    const io = transport()
    await expect(translateImageUnits(imageTranslationPlan(recognition()).units, { ...defaultAiSettings(), provider: 'online-llm' }, false, io)).rejects.toThrow('确认')
    expect(io.translate).not.toHaveBeenCalled()
  })
  it('取消发生在请求期间时不提交迟到结果，也不发送下一行', async () => {
    let cancelled = false
    const io = transport({ cancelled: () => cancelled, translate: vi.fn(async (requestId, _text, _consent, settings) => {
      cancelled = true
      return { requestId, translatedText: 'late', targetLanguage: settings.targetLanguage }
    }) })
    await expect(translateImageUnits(imageTranslationPlan(recognition()).units, defaultAiSettings(), false, io)).rejects.toThrow('取消')
    expect(io.translate).toHaveBeenCalledTimes(1)
    expect(io.completed).not.toHaveBeenCalled()
    expect(io.pending).toHaveBeenLastCalledWith(null)
  })
  it('发送前已关闭面板时不调用翻译服务', async () => {
    const io = transport({ cancelled: () => true })
    await expect(translateImageUnits(imageTranslationPlan(recognition()).units, defaultAiSettings(), false, io)).rejects.toThrow('取消')
    expect(io.translate).not.toHaveBeenCalled()
  })
  it.each(['id', 'target', 'empty'])('拒绝不匹配或空译文：%s', invalid => {
    const io = transport({ translate: vi.fn(async (requestId, _text, _consent, settings) => ({
      requestId: invalid === 'id' ? 'old' : requestId,
      translatedText: invalid === 'empty' ? ' ' : 'text', targetLanguage: invalid === 'target' ? 'invalid' : settings.targetLanguage,
    })) })
    return expect(translateImageUnits(imageTranslationPlan(recognition()).units, defaultAiSettings(), false, io)).rejects.toThrow('不匹配或为空')
  })
  it('中途失败保留前面已提交的结果，清理当前请求并停止发送', async () => {
    let count = 0
    const io = transport({ translate: vi.fn(async (requestId, _text, _consent, settings) => {
      if (++count === 2) throw new Error('service unavailable')
      return { requestId, translatedText: 'saved', targetLanguage: settings.targetLanguage }
    }) })
    await expect(translateImageUnits(imageTranslationPlan(recognition()).units, defaultAiSettings(), false, io)).rejects.toThrow('service unavailable')
    expect(io.completed).toHaveBeenCalledTimes(1)
    expect(io.pending).toHaveBeenLastCalledWith(null)
  })
})
