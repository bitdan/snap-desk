//! 截图覆盖层的识别会话：OCR、翻译交接与二维码结果独立于选区和录屏入口。
//! 结果面板只持有当前截图会话的数据；截图关闭、替换或迟到响应由调用方守卫。
import { nextTick, ref, watch, type Ref } from 'vue'
import { api } from '@/services/api'
import type { OcrResult } from '@/features/recognition/ai'
import type { QrResult } from '@/features/recognition/types'
import type { Annotation } from '@/features/annotations/annotations'
import type { Rect } from '@/shared/geometry/selection'
import type { CaptureFrame } from './types'

interface RecognitionOptions {
  frame: Ref<CaptureFrame | null>
  pixels: Ref<Rect | null>
  busy: Ref<boolean>
  error: Ref<string>
  /** 标注编辑进行中时不得启动新识别，避免结果面板遮挡绘制。 */
  blocked: () => boolean
  gestureActive: () => boolean
  /** 二维码识别沿用当前标注历史，结果面板与导出使用同一份形状。 */
  annotations: () => Annotation[]
  releaseFrame: (id?: number) => void
}

/** 识别与翻译共享可编辑草稿；切换面板不重复 OCR，也不丢失用户修正。 */
export function useCaptureRecognition({ frame, pixels, busy, error, blocked, gestureActive, annotations, releaseFrame }: RecognitionOptions) {
  const qrResult = ref<QrResult | null>(null)
  const ocrResult = ref<OcrResult | null>(null)
  const ocrDraft = ref('')
  /** 截图工具栏的“截图翻译”在识别工作区内就地翻译；从翻译窗口发起的截图仍交回翻译窗口。 */
  const ocrMode = ref<'ocr' | 'translate'>('ocr')
  const openingTranslation = ref(false)
  const ocrEntryWasTranslation = ref(false)
  const translationTrigger = ref<HTMLButtonElement | null>(null)
  const ocrTrigger = ref<HTMLButtonElement | null>(null)

  function closeOcr() {
    ocrResult.value = null
    ocrDraft.value = ''
    void nextTick(() => (ocrEntryWasTranslation.value ? translationTrigger.value : ocrTrigger.value)?.focus())
  }

  async function recognizeText(translate = false) {
    if (!frame.value || !pixels.value || busy.value || blocked() || gestureActive()) return
    const id = frame.value.id
    ocrEntryWasTranslation.value = translate
    busy.value = true; error.value = ''
    try {
      const text = await api.recognizeCapture(id, pixels.value)
      if (frame.value?.id === id) {
        ocrDraft.value = text.fullText
        if (translate && frame.value.translation) { ocrResult.value = text; await sendTranslation(id) }
        else { ocrMode.value = translate ? 'translate' : 'ocr'; ocrResult.value = text }
      }
    } catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  /** 原文交接成功后再释放截图；打开失败时保留 OCR 草稿供重试。 */
  async function sendTranslation(id = frame.value?.id) {
    if (openingTranslation.value) return
    if (id === undefined || !ocrDraft.value.trim()) { error.value = '未识别到文字。'; return }
    openingTranslation.value = true
    try {
      await api.openTranslation(ocrDraft.value)
      await api.cancel(id)
      releaseFrame(id)
    } catch (e) { error.value = String(e) }
    finally { openingTranslation.value = false }
  }

  watch(() => frame.value?.id, () => { qrResult.value = null; ocrResult.value = null; ocrDraft.value = '' })

  /** 主动识别保留截图现场；结果面板隔离选区拖动与截图快捷键。 */
  async function recognizeQr() {
    if (!frame.value || !pixels.value || busy.value || blocked() || gestureActive()) return
    busy.value = true; error.value = ''
    try { qrResult.value = await api.recognizeQr(frame.value.id, pixels.value, annotations()) }
    catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  return {
    qrResult, ocrResult, ocrDraft, openingTranslation,
    translationTrigger, ocrTrigger,
    ocrMode, closeOcr, recognizeText, sendTranslation, recognizeQr,
  }
}
