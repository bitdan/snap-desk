import { onMounted, onUnmounted } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { onRecordingChanged } from './events'
import { createRecordingPoller, recordingPollInterval } from './polling'
import type { RecordingPhase } from './recording'

/** 每个窗口独立管理订阅和计时器；重新显示后先取快照，事件丢失仍有低频校准。 */
export function useRecordingPolling(refresh: () => Promise<void>, phase: () => RecordingPhase | undefined, onError: (error: unknown) => void, enabled = () => true) {
  let disposed = false
  const disposers: (() => void)[] = []
  const nativeWindow = isTauri() ? getCurrentWindow() : null
  const poller = createRecordingPoller({
    refresh, interval: () => recordingPollInterval(phase()), onError,
    visible: async () => {
      if (!enabled()) return false
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return false
      return nativeWindow ? await nativeWindow.isVisible() && !await nativeWindow.isMinimized() : true
    },
  })
  const wake = () => { void poller.wake() }
  async function subscribe(register: () => Promise<() => void>) {
    try {
      const off = await register()
      if (disposed) off(); else disposers.push(off)
    } catch { /* 订阅不可用时仍用快照校准，不阻止录屏控制。 */ }
  }
  onMounted(() => {
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', wake)
    if (nativeWindow) {
      void subscribe(() => onRecordingChanged(wake))
      void subscribe(() => nativeWindow.onFocusChanged(wake))
      void subscribe(() => nativeWindow.onResized(wake))
    }
    wake()
  })
  onUnmounted(() => {
    disposed = true; poller.stop()
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', wake)
    disposers.forEach(off => off())
  })
  return poller
}
