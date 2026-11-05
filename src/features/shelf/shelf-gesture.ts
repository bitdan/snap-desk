/** 单击复制，移动超过 6 CSS 像素交给 OLE；释放捕获后才开始原生拖拽。 */
export function createShelfGesture(copy: () => void, drag: () => void) {
  let pending: { id: number; x: number; y: number; target: HTMLElement } | null = null
  let suppressClick = false
  function cancel() {
    const old = pending; pending = null
    if (old?.target.hasPointerCapture(old.id)) old.target.releasePointerCapture(old.id)
  }
  function down(event: PointerEvent) {
    if (event.button !== 0) return
    cancel(); suppressClick = false
    const target = event.currentTarget as HTMLElement
    pending = { id: event.pointerId, x: event.clientX, y: event.clientY, target }
    target.setPointerCapture(event.pointerId)
  }
  function move(event: PointerEvent) {
    if (!pending || pending.id !== event.pointerId) return
    if (!(event.buttons & 1)) { cancel(); return }
    if (Math.hypot(event.clientX - pending.x, event.clientY - pending.y) < 6) return
    suppressClick = true; cancel(); drag()
  }
  function click(event: MouseEvent) {
    if (suppressClick && event.detail !== 0) { suppressClick = false; return }
    suppressClick = false; copy()
  }
  return { down, move, cancel, click }
}
