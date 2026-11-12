import { ref } from 'vue'
import { pinImageUrl } from '@/shared/image-protocol'
import type { PinFrame, PinImage } from './types'

/** 底图地址由像素版本决定；持续隐藏后清空 src 让 WebView 释放解码图，重新显示时按当前版本再取。 */
export function usePinImage(fetch: () => Promise<PinFrame>) {
  const frame = ref<PinImage | null>(null)
  let visible = true, disposed = false, generation = 0
  let idle: ReturnType<typeof setTimeout> | undefined
  function replace(next: PinFrame) {
    if (disposed || (frame.value && next.revision < frame.value.revision)) return false
    frame.value = { ...next, src: visible ? pinImageUrl(next.label, next.revision) : '' }
    return true
  }
  async function refresh() {
    if (disposed || !visible) return false
    const request = ++generation
    const previous = frame.value?.revision
    let next: PinFrame
    try { next = await fetch() }
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
      idle = setTimeout(() => { if (!visible && frame.value) frame.value.src = '' }, 5000)
    }
  }
  function dispose() { disposed = true; generation++; clearTimeout(idle); frame.value = null }
  return { frame, refresh, replace, visibility, dispose }
}
