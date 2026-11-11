import type { CaptureAfterAction, CaptureFrame } from './types'

/**
 * 框选完成时是否直接执行设置里的动作；返回 null 表示留在工具栏。
 * 回放、录屏和翻译截图各有自己的后续流程；按住 Ctrl 抬起是临时保留工具栏的逃生口，
 * 否则开启直接执行后就无法再标注。
 */
export function autoFinishAction(
  frame: Pick<CaptureFrame, 'afterAction' | 'translation' | 'replay'> | null,
  state: { ctrlKey: boolean; recording: boolean; valid: boolean },
): Exclude<CaptureAfterAction, 'none'> | null {
  const action = frame?.afterAction
  if (!frame || !action || action === 'none') return null
  if (state.ctrlKey || state.recording || frame.translation || frame.replay || !state.valid) return null
  return action
}
