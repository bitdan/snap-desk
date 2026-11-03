import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { dialogWindowCloseUrl, openDetachedWindow } from '../../src/shared/window/detached-window'

const native = vi.hoisted(() => vi.fn(() => false))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: native }))

/** 最小文档宿主：模拟 WebView 导航替换文档和 DOM 节点跨文档迁移。 */
class Element {
  parentNode: Element | null = null
  children: Element[] = []
  attributes = new Map<string, string>()
  className = ''
  appendChild(node: Element) {
    node.parentNode?.removeChild(node)
    node.parentNode = this; this.children.push(node)
    return node
  }
  removeChild(node: Element) { this.children.splice(this.children.indexOf(node), 1); node.parentNode = null }
  setAttribute(name: string, value: string) { this.attributes.set(name, value) }
  getAttribute(name: string) { return this.attributes.get(name) ?? null }
  removeAttribute(name: string) { this.attributes.delete(name) }
  querySelectorAll() { return this.children }
  cloneNode() { return new Element() }
}
class DocumentHost extends EventTarget {
  head = new Element()
  body = new Element()
  documentElement = new Element()
  baseURI = 'http://localhost/index.html'
  title = ''
  activeElement = { isConnected: true, focus: vi.fn() }
  createElement() { return new Element() }
}
function escape(doc: DocumentHost, patch = {}) {
  const event = Object.assign(new Event('keydown', { cancelable: true }), {
    key: 'Escape', keyCode: 27, isComposing: false, repeat: false, ...patch,
  })
  doc.dispatchEvent(event)
  return event
}
function setup() {
  const source = new DocumentHost()
  source.head.appendChild(new Element())
  source.documentElement.setAttribute('data-theme', 'dark')
  const popup = Object.assign(new EventTarget(), {
    document: new DocumentHost(), closed: false, focus: vi.fn(),
    location: { replace: vi.fn() },
    close: vi.fn(() => { popup.closed = true }),
  })
  const owner = Object.assign(new EventTarget(), {
    document: source, screenX: 0, screenY: 0, outerWidth: 1000, outerHeight: 800,
    open: vi.fn(() => popup), setInterval, clearInterval,
  })
  const close = vi.fn(), exit = vi.fn()
  const session = openDetachedWindow(owner as unknown as Window, { title: '预览', width: 900, height: 640 }, close, exit)!
  return { source, popup, owner, close, exit, session }
}
beforeEach(() => {
  native.mockReturnValue(false)
  vi.useFakeTimers()
  vi.stubGlobal('MutationObserver', class { observe() {} disconnect() {} })
})
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })

it('桌面关闭请求交给原生销毁外层窗口，不调用仅关闭 WebView 的 window.close', () => {
  native.mockReturnValue(true)
  const { popup, session, exit } = setup()
  escape(popup.document)
  expect(exit).toHaveBeenCalledOnce()
  session.dispose()
  expect(popup.location.replace).toHaveBeenCalledExactlyOnceWith(dialogWindowCloseUrl)
  expect(popup.close).not.toHaveBeenCalled()
  session.dispose()
  vi.advanceTimersByTime(500)
  expect(popup.location.replace).toHaveBeenCalledOnce()
})

it.each(['load', 'poll'])('首次导航替换文档后通过 %s 恢复预览内容、样式和 Esc', mode => {
  const { popup, exit, session } = setup()
  const initial = popup.document
  const content = new Element()
  ;(session.target as unknown as Element).appendChild(content)
  popup.document = new DocumentHost()
  if (mode === 'load') popup.dispatchEvent(new Event('load'))
  else vi.advanceTimersByTime(250)
  expect(session.target.parentNode).toBe(popup.document.body)
  expect((session.target as unknown as Element).children).toContain(content)
  expect(popup.document.title).toBe('预览')
  expect(popup.document.documentElement.getAttribute('data-theme')).toBe('dark')
  expect(popup.document.head.children).toHaveLength(2)
  escape(initial)
  expect(exit).not.toHaveBeenCalled()
  expect(escape(popup.document).defaultPrevented).toBe(true)
  expect(exit).toHaveBeenCalledOnce()
  session.dispose()
  escape(popup.document)
  expect(exit).toHaveBeenCalledOnce()
  expect(popup.close).toHaveBeenCalledOnce()
  vi.advanceTimersByTime(500)
  expect(popup.document.head.children).toHaveLength(2)
})

it('重复检查不重复绑定 Esc，保留输入法和长按保护，系统关闭后释放会话', () => {
  const { popup, source, close, exit } = setup()
  vi.advanceTimersByTime(1000)
  for (const patch of [{ isComposing: true }, { keyCode: 229 }, { repeat: true }]) escape(popup.document, patch)
  expect(exit).not.toHaveBeenCalled()
  escape(popup.document)
  expect(exit).toHaveBeenCalledOnce()
  popup.closed = true
  vi.advanceTimersByTime(500)
  expect(close).toHaveBeenCalledOnce()
  expect(source.activeElement.focus).toHaveBeenCalledOnce()
  escape(popup.document)
  expect(exit).toHaveBeenCalledOnce()
})
