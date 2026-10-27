import type { RecordingPhase } from './recording'

/** 活跃录制只校准时间/统计；阶段切换由事件立即刷新，终态降低查询频率。 */
export function recordingPollInterval(phase?: RecordingPhase): number {
  if (phase === 'countdown') return 250
  if (phase === 'recording' || phase === 'encoding') return 1000
  if (phase === 'ready' || phase === 'paused') return 2000
  return 10000
}

/** 串行调度器：事件合并为一次补刷，隐藏时仅低频检查可见性，不读取业务状态。 */
export function createRecordingPoller(options: {
  refresh: () => Promise<void>; visible: () => Promise<boolean>; interval: () => number
  onError: (error: unknown) => void
}) {
  let stopped = false, running = false, pending = false
  let timer: ReturnType<typeof setTimeout> | undefined
  async function wake() {
    if (stopped) return
    clearTimeout(timer)
    if (running) { pending = true; return }
    running = true
    let visible = false
    try {
      visible = await options.visible()
      if (!stopped && visible) await options.refresh()
    } catch (error) { if (!stopped) options.onError(error) }
    finally {
      running = false
      if (!stopped) {
        const immediate = pending && visible
        pending = false
        timer = setTimeout(() => { void wake() }, immediate ? 0 : visible ? options.interval() : 10000)
      }
    }
  }
  return { wake, stop() { stopped = true; clearTimeout(timer) } }
}
