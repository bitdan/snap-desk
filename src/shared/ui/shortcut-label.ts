/**
 * 快捷键的界面展示。配置中统一保存为 `Ctrl+Alt+Shift+Super+键`（与原生注册格式一致），
 * 这里只改变显示：macOS 使用 ⌃⌥⇧⌘ 符号并按系统惯例紧挨书写，其他平台保持原文。
 */
import { macPlatform } from '@/shared/platform'

export { macPlatform }

const macSymbols: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' }

/** 拆成逐键显示的标签；兼容录入预览中的 `Ctrl + Shift` 写法。 */
export function shortcutKeys(value: string, mac = macPlatform): string[] {
  const keys = value.split(/\s*\+\s*/).filter(Boolean)
  return mac ? keys.map(key => macSymbols[key] ?? key) : keys
}

/** 单段文字展示，例如 macOS 的 `⌥V`；其他平台原样返回，不改变既有界面。 */
export function shortcutText(value: string, mac = macPlatform): string {
  return mac ? shortcutKeys(value, true).join('') : value
}
