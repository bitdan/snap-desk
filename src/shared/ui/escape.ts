/** 窗口统一的 Esc 入口：输入框也可退出，内层已处理、输入法组词和长按不重复触发。 */
export function handleEscape(event: KeyboardEvent, exit: () => void): boolean {
  if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.repeat) return false
  event.preventDefault()
  exit()
  return true
}
