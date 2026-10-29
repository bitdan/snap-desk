import { ref } from 'vue'
import type { PinFrame } from './types'

/** 可见窗口按像素版本复用图片；短暂隐藏保留缓存，持续隐藏后移除 Data URL 与解码入口。 */
export function usePinImage(fetch: (knownRevision: number | null) => Promise<PinFrame>) {
  const frame = ref<PinFrame | null>(null)
  let visible = true, disposed = false, generation = 0
  let idle: ReturnType<typeof setTimeout> | undefined
  function replace(next: PinFrame) {
    if (disposed || (frame.value && next.revision < frame.value.revision)) return false
    const cached = frame.value?.revision === next.revision ? frame.value.dataUrl : ''
    if (visible && !next.dataUrl && !cached) throw new Error('贴图图片缓存已失效，请重新显示贴图。')
    frame.value = { ...next, dataUrl: visible ? next.dataUrl || cached : '' }
    return true
  }
  async function refresh() {
    if (disposed || !visible) return false
    const request = ++generation
    const previous = frame.value?.revision
    let next: PinFrame
    try { next = await fetch(frame.value?.dataUrl ? frame.value.revision : null) }
    catch (error) {
      if (disposed || !visible || request !== generation) return false
      throw error
    }
    if (disposed || !visible || request !== generation) return false
    if (!replace(next)) return false
    return previous !== next.revision
  }
  function visibility(value: boolean) {
    if (disposed) return
    visible = value; generation++; clearTimeout(idle)
    if (!value) {
      // 系统文件对话框会短暂隐藏贴图；五秒后再释放，兼顾快速切回与长期隐藏占用。
      idle = setTimeout(() => { if (!visible && frame.value) frame.value.dataUrl = '' }, 5000)
    }
  }
  function dispose() { disposed = true; generation++; clearTimeout(idle); frame.value = null }
  return { frame, refresh, replace, visibility, dispose }
}
