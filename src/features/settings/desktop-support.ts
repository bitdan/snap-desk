import type { DesktopFeatures, DesktopSupport } from '@/features/settings/types'

export type DesktopFeatureKey = keyof DesktopFeatures

/** 缺少报告（旧后端或测试替身）时按可用处理，保持原有行为。 */
export function featureAvailable(support: DesktopSupport | null | undefined, key: DesktopFeatureKey): boolean {
  return support?.features?.[key]?.available !== false
}

/** 各页面受当前桌面限制的功能；同一原因只显示一次。 */
const pageFeatures: Record<string, DesktopFeatureKey[]> = {
  '截图': ['screenshot', 'screenshotCursor', 'windowSnap', 'scrollCapture'],
  '录屏': ['recording', 'recordingEffects'],
  '贴图': ['pinOnTop'],
  '剪贴板': ['clipboardHistory', 'directPaste'],
  '识别与翻译': ['selectedText'],
  '快捷键': ['globalShortcuts'],
}

export function pageLimitations(support: DesktopSupport | null | undefined, page: string): string[] {
  const reasons = (pageFeatures[page] ?? [])
    .map(key => support?.features?.[key])
    .filter(feature => feature && !feature.available && feature.reason)
    .map(feature => feature!.reason!)
  return [...new Set(reasons)]
}

/**
 * 供用户在桌面或合成器中绑定的命令（已由原生端按 shell 规则加引号）。
 * Wayland 始终提供：门户绑定可能被桌面拒绝或改键，命令行入口是可靠的后备。
 */
export function shortcutCommands(support: DesktopSupport | null | undefined, actions: readonly { key: string; label: string }[]): { label: string; command: string }[] {
  if (!support || (support.session !== 'wayland' && featureAvailable(support, 'globalShortcuts'))) return []
  return actions.map(action => ({ label: action.label, command: `${support.commandLine} --${action.key}` }))
}
