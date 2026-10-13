/** 原生标签优先，避免辅助窗口重新加载后误挂成主页。 */
export function resolveWindowKind(label: string | null, search: string): string {
  if (label) {
    if (/^(scroll|recording)-edge-(top|bottom|left|right)$/.test(label)) return 'selection-edge'
    if (label.startsWith('pin-')) return 'pin'
    return label
  }
  return new URLSearchParams(search).get('window') ?? 'main'
}
