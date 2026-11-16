//! 截图覆盖层的识别会话：OCR、翻译交接与二维码结果独立于选区和录屏入口。
//! 结果面板只持有当前截图会话的数据；截图关闭、替换或迟到响应由调用方守卫。
import { nextTick, ref, shallowRef, watch, type Ref } from 'vue'
import { api } from '@/services/api'
import type { OcrResult } from '@/features/recognition/ai'
import { initialOcrText, type OcrImage } from '@/features/recognition/ocr-workspace'
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
  /** 截图会话释放前取得选区原图的内存副本；取不到时结果窗口只显示文字。 */
  snapshot: (rect: Rect) => OcrImage | null
}

/** 识别与翻译共享可编辑草稿；切换面板不重复 OCR，也不丢失用户修正。 */
export function useCaptureRecognition({ frame, pixels, busy, error, blocked, gestureActive, annotations, releaseFrame, snapshot }: RecognitionOptions) {
  const qrResult = ref<QrResult | null>(null)
  const ocrResult = ref<OcrResult | null>(null)
  const ocrDraft = ref('')
  /** 截图工具栏的“截图翻译”在识别工作区内就地翻译；从翻译窗口发起的截图仍交回翻译窗口。 */
  const ocrMode = ref<'ocr' | 'translate'>('ocr')
  const openingTranslation = ref(false)
  const ocrEntryWasTranslation = ref(false)
  const translationTrigger = ref<HTMLButtonElement | null>(null)
  const ocrTrigger = ref<HTMLButtonElement | null>(null)

  /** 识别出内容后整屏蒙层即收起，结果窗口脱离截图会话独立存活，直到用户关闭或开始下一次截图。 */
  const ocrDetached = ref(false)
  const qrDetached = ref(false)
  const ocrSnapshot = shallowRef<OcrImage | null>(null)

  function resetOcr() {
    ocrResult.value = null
    ocrDraft.value = ''
    ocrDetached.value = false
    ocrSnapshot.value = null
  }

  function closeQr() {
    qrResult.value = null
    qrDetached.value = false
  }

  function closeOcr() {
    resetOcr()
    void nextTick(() => (ocrEntryWasTranslation.value ? translationTrigger.value : ocrTrigger.value)?.focus())
  }

  async function recognizeText(translate = false) {
    if (!frame.value || !pixels.value || busy.value || blocked() || gestureActive()) return
    const id = frame.value.id, rect = { ...pixels.value }
    ocrEntryWasTranslation.value = translate
    busy.value = true; error.value = ''
    try {
      const text = await api.recognizeCapture(id, rect)
      if (frame.value?.id === id) {
        ocrDraft.value = initialOcrText(text)
        if (translate && frame.value.translation) { ocrResult.value = text; await sendTranslation(id) }
        else {
          ocrMode.value = translate ? 'translate' : 'ocr'
          // 没识别到文字时保留截图现场，便于直接重新框选。
          if (ocrDraft.value.trim()) {
            ocrSnapshot.value = snapshot(rect)
            if (!await detach(id, ocrDetached)) ocrSnapshot.value = null
          }
          if (ocrDetached.value || frame.value?.id === id) ocrResult.value = text
        }
      }
    } catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  /** 先结束截图会话再打开结果窗口，后端收尾不会取消窗口内随后发起的翻译；收起失败则退回蒙层内显示。 */
  async function detach(id: number, detached: Ref<boolean>) {
    detached.value = true
    try {
      await api.cancel(id)
      releaseFrame(id)
    } catch { detached.value = false }
    return detached.value
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

  // 脱离会话的结果窗口不随截图释放而关闭；新截图到来时让位。
  watch(() => frame.value?.id, id => {
    if (id !== undefined || !qrDetached.value) closeQr()
    if (id !== undefined || !ocrDetached.value) resetOcr()
  })

  /** 识别到二维码后收起蒙层，结果窗口独立保留；没有识别到时保留截图现场供重新框选。 */
  async function recognizeQr() {
    if (!frame.value || !pixels.value || busy.value || blocked() || gestureActive()) return
    const id = frame.value.id
    busy.value = true; error.value = ''
    try {
      const result = await api.recognizeQr(id, pixels.value, annotations())
      if (frame.value?.id !== id) return
      if (result.items.length) await detach(id, qrDetached)
      if (qrDetached.value || frame.value?.id === id) qrResult.value = result
    }
    catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  return {
    qrResult, qrDetached, ocrResult, ocrDraft, ocrDetached, ocrSnapshot, openingTranslation,
    translationTrigger, ocrTrigger,
    ocrMode, closeOcr, closeQr, recognizeText, sendTranslation, recognizeQr,
  }
}
