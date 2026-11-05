//! 截图覆盖层的导出动作：复制 / 保存 / 贴图、取消以及滚动截图接管。
//! 只负责调用顺序与失败提示；截图帧释放和最近选区记录由宿主会话维护。
import type { Ref } from 'vue'
import { api } from '@/services/api'
import type { Annotation } from '@/features/annotations/annotations'
import type { Rect } from '@/shared/geometry/selection'
import type { CaptureFrame } from './types'

interface ActionOptions {
  frame: Ref<CaptureFrame | null>
  pixels: Ref<Rect | null>
  valid: Ref<boolean | null>
  busy: Ref<boolean>
  error: Ref<string>
  scroll: Ref<unknown | null>
  /** 标注交互或选区拖动进行中时拒绝新的整图操作。 */
  blocked: () => boolean
  marked: () => boolean
  /** 取消只等待文字保存，不阻止绘制中的 Esc；与原有交互语义保持一致。 */
  editorPending: () => boolean
  /** 识别/翻译结果面板打开时，底层导出快捷键不得抢占。 */
  recognitionOpen: () => boolean
  annotations: () => Annotation[]
  releaseFrame: (id?: number) => void
  remember: (rect: Rect, image: { width: number; height: number; layoutKey?: string }) => void
}

export function useCaptureActions({
  frame, pixels, valid, busy, error, scroll, blocked, marked, editorPending,
  recognitionOpen, annotations, releaseFrame, remember,
}: ActionOptions) {
  async function finish(action: 'copy' | 'save' | 'pin') {
    if (!frame.value || !valid.value || !pixels.value || recognitionOpen() || blocked()) return
    const id = frame.value.id
    busy.value = true; error.value = ''
    try {
      // 保存调用前的快照；取消对话框或导出失败均不覆盖上次成功的范围。
      const rect = { ...pixels.value }
      const image = { width: frame.value.width, height: frame.value.height, layoutKey: frame.value.layoutKey }
      const result = action === 'copy'
        ? await api.copy(id, rect, annotations())
        : action === 'save'
          ? await api.save(id, rect, annotations())
          : await api.pin(id, rect, annotations())
      if (result !== false) {
        remember(rect, image)
        releaseFrame(id)
      }
    } catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  async function cancel() {
    if (!frame.value || busy.value || editorPending()) return
    const id = frame.value.id
    busy.value = true
    try { await api.cancel(id); releaseFrame(id) } catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  async function startScroll() {
    if (!frame.value || frame.value.replay || !pixels.value || !valid.value || blocked() || marked()) return
    busy.value = true; error.value = ''
    // 滚动截图仅输出拼接结果；排错使用应用日志，不再额外保存原始帧。
    const id = frame.value.id
    try {
      await api.startScroll(id, pixels.value, false, 'white')
      // 独立预览窗口接管进度；隐藏浮层不保留整桌面图片或标注历史。
      releaseFrame(id)
      if (!frame.value) scroll.value = null
    }
    catch (e) { error.value = String(e) }
    finally { busy.value = false }
  }

  return { finish, cancel, startScroll }
}
