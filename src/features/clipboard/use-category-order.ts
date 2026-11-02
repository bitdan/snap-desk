/** 分类顺序在当前设备保存；指针排序兼容桌面 WebView 的原生文件拖放。 */
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import type { ClipboardKind, ClipboardTextCategory } from './clipboard'

export type ClipboardCategory = '' | ClipboardKind | ClipboardTextCategory | 'favorites'
const defaults: ClipboardCategory[] = ['', 'text', 'image', 'file', 'link', 'path', 'color', 'favorites']
const storageKey = 'snap-desk.clipboard-category-order.v1'

function readOrder(): ClipboardCategory[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey) ?? 'null')
    if (Array.isArray(saved)) {
      const valid = [...new Set(saved.filter((key): key is ClipboardCategory => defaults.includes(key)))]
      return [...valid, ...defaults.filter(key => !valid.includes(key))]
    }
  } catch { /* 无效存储回到默认顺序。 */ }
  return [...defaults]
}

/** 拖动超过 5 CSS 像素才排序；普通点击仍然切换分类。 */
export function useCategoryOrder(disabled: () => boolean, select: (key: ClipboardCategory) => void) {
  const order = ref(readOrder()), sorting = ref<ClipboardCategory | null>(null), target = ref<ClipboardCategory | null>(null)
  const orderError = ref('')
  let drag: { key: ClipboardCategory; id: number; x: number; y: number; moved: boolean; button: HTMLElement } | null = null
  let suppressClick = false
  function save(source: ClipboardCategory, destination: ClipboardCategory) {
    if (source === destination) return
    const next = [...order.value], index = next.indexOf(source), to = next.indexOf(destination)
    if (index < 0 || to < 0) return
    next.splice(index, 1); next.splice(to, 0, source); order.value = next
    try { localStorage.setItem(storageKey, JSON.stringify(next)); orderError.value = '' }
    catch { orderError.value = '分类顺序仅在当前窗口生效，无法保存。' }
  }
  function start(event: PointerEvent, key: ClipboardCategory) {
    if (disabled() || event.button !== 0 || drag) return
    suppressClick = false
    const button = event.currentTarget as HTMLElement
    drag = { key, id: event.pointerId, x: event.clientX, y: event.clientY, moved: false, button }
    button.setPointerCapture(event.pointerId)
  }
  function hit(event: PointerEvent) {
    const button = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-clipboard-category]')
    return button && button.parentElement === drag?.button.parentElement ? button : null
  }
  function move(event: PointerEvent) {
    if (!drag || drag.id !== event.pointerId) return
    if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) > 5) drag.moved = true
    if (drag.moved) {
      sorting.value = drag.key
      target.value = hit(event)?.dataset.clipboardCategory as ClipboardCategory | undefined ?? null
    }
  }
  function cancel(event?: PointerEvent) {
    if (!drag || (event && event.pointerId !== drag.id)) return
    const source = drag
    drag = null; sorting.value = null; target.value = null; suppressClick = source.moved
    if (source.button.hasPointerCapture(source.id)) source.button.releasePointerCapture(source.id)
  }
  function finish(event: PointerEvent) {
    if (!drag || drag.id !== event.pointerId) return
    move(event)
    const source = drag, destination = hit(event)?.dataset.clipboardCategory as ClipboardCategory | undefined
    cancel()
    if (source.moved && destination !== undefined && !disabled()) save(source.key, destination)
  }
  function choose(event: MouseEvent, key: ClipboardCategory) {
    if (suppressClick && event.detail !== 0) { suppressClick = false; return }
    suppressClick = false
    if (!disabled()) select(key)
  }
  function key(event: KeyboardEvent, category: ClipboardCategory) {
    if (event.key === 'Escape' && drag) {
      event.preventDefault(); event.stopPropagation(); cancel(); return
    }
    if (!event.altKey || !['ArrowLeft', 'ArrowRight'].includes(event.key) || disabled()) return
    event.preventDefault(); event.stopPropagation()
    const destination = order.value[order.value.indexOf(category) + (event.key === 'ArrowLeft' ? -1 : 1)]
    if (destination !== undefined) {
      const button = event.currentTarget as HTMLElement
      save(category, destination)
      void nextTick(() => button.focus({ preventScroll: true }))
    }
  }
  function blur() { cancel() }
  onMounted(() => window.addEventListener('blur', blur))
  onUnmounted(() => { cancel(); window.removeEventListener('blur', blur) })
  return { order, sorting, target, orderError, start, move, finish, cancel, choose, key }
}
