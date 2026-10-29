/** 窗口统一的 Esc 入口：输入框也可退出，内层已处理、输入法组词和长按不重复触发。 */
export function handleEscape(event: KeyboardEvent, exit: () => void): boolean {
  if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.repeat) return false
  event.preventDefault()
  exit()
  return true
}

const layers: { active: () => boolean; close: () => void }[] = []
/** 每个 WebView 独立维护活动弹层，后打开的层优先；注销函数用于关闭和卸载清理。 */
export function registerEscapeLayer(active: () => boolean, close: () => void): () => void {
  const layer = { active, close }
  layers.push(layer)
  return () => { const index = layers.indexOf(layer); if (index >= 0) layers.splice(index, 1) }
}

/** 窗口兜底先关闭最上层，避免焦点移到弹层外后 Esc 直接收起主窗口或结束录制。 */
export function handleWindowEscape(event: KeyboardEvent, exit: () => void): boolean {
  const top = [...layers].reverse().find(layer => layer.active())
  return handleEscape(event, top?.close ?? exit)
}
