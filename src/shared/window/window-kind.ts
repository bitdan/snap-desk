/** 原生窗口标签是固定契约，避免辅助窗口导航/重载后查询参数丢失挂错页面。 */
export const windowKinds = [
  'main',
  'capture',
  'pin',
  'shelf',
  'clipboard',
  'translation',
  'recording-controls',
  'scroll-preview',
  'scroll-controls',
  'selection-edge',
] as const
export type WindowKind = typeof windowKinds[number]

/** 原生标签优先，避免辅助窗口重新加载后误挂成主页；未知标签回落到主页。 */
export function resolveWindowKind(label: string | null, search: string): WindowKind {
  if (label) {
    if (/^(scroll|recording)-edge-(top|bottom|left|right)$/.test(label)) return 'selection-edge'
    if (label.startsWith('pin-')) return 'pin'
    return windowKinds.includes(label as WindowKind) ? label as WindowKind : 'main'
  }
  const kind = new URLSearchParams(search).get('window')
  return windowKinds.includes(kind as WindowKind) ? kind as WindowKind : 'main'
}
