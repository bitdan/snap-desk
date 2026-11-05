//! 截图覆盖层的录屏入口：模式切换、能力查询、准备条偏好与接管失败清理。
//! 截图选区仍由宿主维护；这里只消费确认后的原图选区，不重新读取或缓存桌面画面。
import { nextTick, ref, type ComputedRef, type Ref } from 'vue'
import { api } from '@/services/api'
import type { Annotation } from '@/features/annotations/annotations'
import type { Rect } from '@/shared/geometry/selection'
import {
  recordingBitrate, recordingConfig, recordingRegion,
  type RecordingSelection,
} from '@/features/recording/recording'
import { prepareRecordingSession, useRecordingSetup } from '@/features/recording/use-recording-setup'
import type { CaptureFrame } from './types'

/** 录屏标注窗口只需暴露准备桌面画布的入口，避免截图宿主依赖其内部组件类型。 */
export interface LiveRecordingAnnotations {
  prepareDesktop(): Promise<void>
}

interface RecordingOptions {
  frame: Ref<CaptureFrame | null>
  pixels: Ref<Rect | null>
  valid: ComputedRef<boolean | null>
  busy: Ref<boolean>
  error: Ref<string>
  annotating: Ref<boolean>
  marked: ComputedRef<boolean>
  activeDisplay: ComputedRef<{ x: number; y: number; width: number; height: number }>
  selection: Ref<Rect | null>
  /** 模式由宿主提前创建，选区会话在同一次 setup 中即可读取。 */
  mode: Ref<boolean>
  gestureActive: () => boolean
  editorBlocked: () => boolean
  annotations: () => Annotation[]
  releaseFrame: (id?: number) => void
  /** 切换录屏范围时清除截图窗口吸附，避免旧目标与新选区混用。 */
  clearTarget: () => void
  /** 覆盖层已卸载时丢弃迟到的能力响应。 */
  disposed: () => boolean
  liveRecordingId: Ref<number | null>
  liveAnnotations: Ref<LiveRecordingAnnotations | null>
}

/** 录屏使用独立准备条；进入后截图标注与导出操作全部隐藏，退出恢复截图焦点。 */
export function useCaptureRecording({
  frame, pixels, valid, busy, error, annotating, marked, activeDisplay, selection, mode,
  gestureActive, editorBlocked, annotations, releaseFrame, clearTarget, disposed, liveRecordingId, liveAnnotations,
}: RecordingOptions) {
  const recordingTrigger = ref<HTMLButtonElement | null>(null)
  const recordingConfirmTrigger = ref<HTMLButtonElement | null>(null)
  const {
    recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps,
    recordingCountdown, captureCursor, recordingEffects, recordingSystemAudio, recordingMicrophone,
    recordingCapabilities, recordingCapabilityError, hideMainWindow, reloadPreferences,
  } = useRecordingSetup()
  const checkingRecordingCapabilities = ref(false)
  const recordingSource = ref<RecordingSelection>('region')

  /** 普通截图与长截图不需要录屏能力；首次进入录屏才读取，失败和不支持均可主动重试。 */
  async function loadRecordingCapabilities(retry = false) {
    if (disposed() || checkingRecordingCapabilities.value || (!retry && recordingCapabilities.value)) return
    checkingRecordingCapabilities.value = true; recordingCapabilityError.value = ''; recordingCapabilities.value = null
    try {
      const value = await api.recordingCapabilities()
      if (!disposed()) recordingCapabilities.value = value
    } catch (e) { if (!disposed()) recordingCapabilityError.value = String(e) }
    finally { if (!disposed()) checkingRecordingCapabilities.value = false }
  }

  /** F3 与截图中的录屏按钮使用同一模式；调整范围后保留录屏偏好。 */
  function enterRecordingMode() {
    if (busy.value || editorBlocked() || frame.value?.replay) return
    annotating.value = false; error.value = ''; mode.value = true
    void loadRecordingCapabilities()
    void nextTick(() => recordingConfirmTrigger.value?.focus())
  }

  function leaveRecordingMode() {
    if (busy.value) return
    mode.value = false; error.value = ''
    void nextTick(() => recordingTrigger.value?.focus())
  }

  /** 录屏仅选择屏幕区域或整台显示器，不再置前或绑定应用窗口。 */
  function chooseRecordingSource(value: string) {
    if (!frame.value || busy.value || marked.value || editorBlocked()) return
    if (value !== 'display' && value !== 'region') return
    clearTarget()
    if (value === 'display') selection.value = { ...activeDisplay.value }
    recordingSource.value = value
  }

  async function selectedRecordingRegion() {
    if (!mode.value || !valid.value) return
    await nextTick()
    recordingConfirmTrigger.value?.focus()
  }

  /** 确认原图像素选区并进入 Ready；后端接管后等待开始按钮或空格，不自动采集。 */
  async function startRecording() {
    if (!mode.value || !recordingCapabilities.value?.available) return
    if (!frame.value || !pixels.value || !valid.value || busy.value || editorBlocked() || gestureActive()) return
    const id = frame.value.id
    busy.value = true; error.value = ''
    try {
      const rect = recordingRegion(pixels.value, recordingSource.value, frame.value.displays, { x: frame.value.originX, y: frame.value.originY })
      const config = recordingConfig(rect, recordingFps.value, captureCursor.value, recordingCountdown.value, recordingEncoding.value, recordingQuality.value, recordingEffects.value, recordingFormat.value === 'gif' ? null : recordingBitrate(recordingMbps.value), recordingFormat.value, recordingSystemAudio.value && !!recordingCapabilities.value?.systemAudio, recordingMicrophone.value && !!recordingCapabilities.value?.microphone, hideMainWindow.value)
      config.source = 'selectedMonitor'
      await prepareRecordingSession(
        () => api.startRecording(id, rect, config, undefined, annotations()),
        async () => {
          if (!liveAnnotations.value) throw new Error('录屏工具栏尚未就绪。')
          await liveAnnotations.value.prepareDesktop()
        },
        () => api.controlRecording(id, 'cancel'),
        () => { liveRecordingId.value = id; releaseFrame(id) },
      )
    } catch (e) {
      // 这里只进入 Ready，开始由录屏工具栏按钮或空格触发；接管失败则释放会话。
      error.value = String(e)
    } finally { busy.value = false }
  }

  return {
    recordingTrigger, recordingConfirmTrigger, recordingSource,
    recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps,
    recordingCountdown, captureCursor, recordingEffects, recordingSystemAudio, recordingMicrophone,
    recordingCapabilities, recordingCapabilityError, hideMainWindow, checkingRecordingCapabilities,
    reloadPreferences,
    loadRecordingCapabilities, enterRecordingMode, leaveRecordingMode, chooseRecordingSource,
    selectedRecordingRegion, startRecording,
  }
}
