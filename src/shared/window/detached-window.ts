/** 独立窗口与业务共用 Vue 状态；只移动 DOM，不向新 WebView 复制业务数据或开放 IPC。 */
import { isTauri } from '@tauri-apps/api/core'

export const dialogWindowUrl = 'about:blank#snap-desk-dialog'
// 使用 query 改变文档地址；仅改变 hash 可能不触发 WebView2 的原生导航拦截。
export const dialogWindowCloseUrl = 'about:blank?snap-desk-dialog-close'

export interface DetachedWindowOptions { title: string; width: number; height: number }
export interface DetachedWindowSession { window: Window; target: HTMLElement; dispose: () => void }

/** about:blank 继承打开者的同源环境，原生端只接收这一固定地址的开窗请求。 */
export function openDetachedWindow(owner: Window, options: DetachedWindowOptions, closed: () => void, escape: () => void): DetachedWindowSession | null {
  const left = Math.round(owner.screenX + Math.max(0, (owner.outerWidth - options.width) / 2))
  const top = Math.round(owner.screenY + Math.max(0, (owner.outerHeight - options.height) / 2))
  const opened = owner.open(dialogWindowUrl, '_blank', `popup=yes,width=${options.width},height=${options.height},left=${left},top=${top}`)
  if (!opened) return null
  const popup: Window = opened
  const source = owner.document
  let doc = popup.document
  const target = doc.createElement('div')
  target.className = 'detached-window-surface'
  // 业务组件可能按需加载样式；同步后续样式和外观，避免弹窗使用过期主题。
  const styleNodes: Node[] = []
  function syncStyles() {
    for (const node of styleNodes.splice(0)) node.parentNode?.removeChild(node)
    source.head.querySelectorAll('style, link[rel="stylesheet"]').forEach(node => {
      const copy = node.cloneNode(true)
      doc.head.appendChild(copy); styleNodes.push(copy)
    })
    for (const name of ['data-theme', 'data-font-size', 'lang', 'style']) {
      const value = source.documentElement.getAttribute(name)
      if (value === null) doc.documentElement.removeAttribute(name)
      else doc.documentElement.setAttribute(name, value)
    }
    doc.documentElement.className = 'detached-window'
  }
  const styles = new MutationObserver(syncStyles)
  styles.observe(source.head, { childList: true, subtree: true, characterData: true, attributes: true })
  styles.observe(source.documentElement, { attributes: true })
  let released = false
  const focused = source.activeElement as HTMLElement | null
  const key = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && !event.defaultPrevented && !event.isComposing && event.keyCode !== 229 && !event.repeat) {
      event.preventDefault(); escape()
    }
  }
  /** WebView 的首次导航可能替换同步取得的空白文档；保留 Teleport 容器并迁移到新文档。 */
  function mountDocument() {
    if (released || popup.closed) return
    const current = popup.document
    if (!current.body) return
    if (current === doc && target.parentNode === current.body) return
    doc.removeEventListener('keydown', key)
    doc = current
    const base = doc.createElement('base')
    base.setAttribute('href', source.baseURI)
    doc.head.appendChild(base)
    doc.title = options.title
    doc.body.appendChild(target)
    doc.addEventListener('keydown', key)
    syncStyles()
  }
  mountDocument()
  popup.addEventListener('load', mountDocument)
  // 系统关闭按钮与浏览器关闭都走同一回调；父窗口释放时同步关闭子窗口。
  // 导航也可能清掉初始 Window 上的 load 监听，关闭轮询同时兜底恢复文档。
  const timer = owner.setInterval(() => {
    if (popup.closed && !released) { dispose(); closed() }
    else mountDocument()
  }, 250)
  function dispose() {
    if (released) return
    released = true; owner.clearInterval(timer); styles.disconnect()
    doc.removeEventListener('keydown', key)
    popup.removeEventListener('load', mountDocument)
    owner.removeEventListener('pagehide', dispose)
    if (!popup.closed) {
      // WebView 的 window.close() 不可靠地销毁外层系统窗口；固定导航由原生拦截并销毁自身。
      if (isTauri()) popup.location.replace(dialogWindowCloseUrl)
      else popup.close()
    }
    if (focused?.isConnected) focused.focus()
  }
  owner.addEventListener('pagehide', dispose)
  popup.focus()
  return { window: popup, target, dispose }
}
