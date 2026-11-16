import { macPlatform } from '@/shared/platform'

/**
 * 拖出修饰键在按下时确定意图；超过阈值后只交给系统拖拽，松开修饰键不改变会话。
 * Windows 用 Ctrl；macOS 的 Ctrl+点击是右键菜单，改用 ⌥（与访达“拖动时复制”一致）。
 */
export function createPinDrag(moveWindow: () => void, exportContent: () => void, mac = macPlatform) {
  let pending: { id: number; x: number; y: number; target: HTMLElement } | null = null
  function cancel() {
    const previous = pending; pending = null
    if (previous?.target.hasPointerCapture(previous.id)) previous.target.releasePointerCapture(previous.id)
  }
  function down(event: PointerEvent) {
    if (event.button !== 0) return
    cancel()
    if (!(mac ? event.altKey : event.ctrlKey)) { moveWindow(); return }
    event.preventDefault(); event.stopPropagation()
    const target = event.currentTarget as HTMLElement
    pending = { id: event.pointerId, x: event.clientX, y: event.clientY, target }
    target.setPointerCapture(event.pointerId)
  }
  function move(event: PointerEvent) {
    if (!pending || event.pointerId !== pending.id) return
    if (!(event.buttons & 1)) { cancel(); return }
    if (Math.hypot(event.clientX - pending.x, event.clientY - pending.y) < 6) return
    event.preventDefault(); event.stopPropagation()
    // 先释放 WebView 指针捕获，再开始系统 OLE 拖拽，避免两种鼠标捕获互相干扰。
    cancel()
    exportContent()
  }
  return { down, move, cancel }
}
