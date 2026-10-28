import { nextTick, onScopeDispose, ref, watch } from 'vue'
import type { RecorderCapabilities, RecordingFps } from './recording'
import { readRecordingPreferences, saveRecordingPreferences } from './preferences'

/** 录屏准备条拥有自己的偏好、格式切换与能力校验，截图宿主只消费这些状态。 */
export function useRecordingSetup() {
  const saved = readRecordingPreferences()
  const recordingFormat = ref(saved.format), recordingFps = ref<RecordingFps>(saved.format === 'gif' ? saved.gifFps : saved.mp4Fps)
  const recordingEncoding = ref(saved.encoding), recordingQuality = ref(saved.quality)
  const recordingMbps = ref<string | number>(saved.mbps ?? ''), recordingCountdown = ref(saved.countdown)
  const captureCursor = ref(saved.captureCursor), recordingEffects = ref(saved.effects)
  const hideMainWindow = ref(saved.hideMainWindow)
  // 音频权限和采集意图不从缓存恢复；每次 WebView 启动都需要用户显式开启。
  const recordingSystemAudio = ref(false), recordingMicrophone = ref(false)
  const recordingCapabilities = ref<RecorderCapabilities | null>(null), recordingCapabilityError = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined
  watch(recordingFormat, (format, previous) => {
    if (previous === 'gif') saved.gifFps = recordingFps.value; else saved.mp4Fps = recordingFps.value
    recordingFps.value = format === 'gif' ? saved.gifFps : saved.mp4Fps
  }, { flush: 'sync' })
  watch(recordingCapabilities, capabilities => {
    if (capabilities?.formats.length && !capabilities.formats.includes(recordingFormat.value)) recordingFormat.value = capabilities.formats[0]!
  })
  function persist() {
    clearTimeout(timer)
    const fps = recordingFps.value
    if (recordingFormat.value === 'gif') saved.gifFps = fps; else saved.mp4Fps = fps
    const mbps = recordingMbps.value === '' ? null : Number(recordingMbps.value)
    saveRecordingPreferences({ ...saved, format: recordingFormat.value, encoding: recordingEncoding.value, quality: recordingQuality.value,
      mbps, countdown: recordingCountdown.value, captureCursor: captureCursor.value, effects: recordingEffects.value, hideMainWindow: hideMainWindow.value })
  }
  watch([recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps, recordingCountdown, captureCursor, recordingEffects, hideMainWindow], () => {
    clearTimeout(timer); timer = setTimeout(persist, 200)
  }, { deep: true })
  onScopeDispose(persist)
  return { recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps, recordingCountdown, captureCursor,
    recordingEffects, recordingSystemAudio, recordingMicrophone, recordingCapabilities, recordingCapabilityError, hideMainWindow }
}

/** 确认后先交出冻结背景，再在同一 WebView 激活控制条；任何接管失败都取消已准备的会话。 */
export async function prepareRecordingSession(
  prepare: () => Promise<void>, activate: () => Promise<void>, cancel: () => Promise<void>, onPrepared: () => void,
) {
  let prepared = false
  try {
    await prepare(); prepared = true
    onPrepared(); await nextTick(); await activate()
  } catch (error) {
    if (prepared) { try { await cancel() } catch { /* 保留最初失败原因。 */ } }
    throw error
  }
}
