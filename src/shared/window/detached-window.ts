/** 独立窗口与业务共用 Vue 状态；只移动 DOM，不向新 WebView 复制业务数据或开放 IPC。 */
export const dialogWindowUrl = 'about:blank#snap-desk-dialog'

export interface DetachedWindowOptions { title: string; width: number; height: number }
export interface DetachedWindowSession { window: Window; target: HTMLElement; dispose: () => void }

/** about:blank 继承打开者的同源环境，原生端只接收这一固定地址的开窗请求。 */
export function openDetachedWindow(owner: Window, options: DetachedWindowOptions, closed: () => void, escape: () => void): DetachedWindowSession | null {
  const left = Math.round(owner.screenX + Math.max(0, (owner.outerWidth - options.width) / 2))
  const top = Math.round(owner.screenY + Math.max(0, (owner.outerHeight - options.height) / 2))
  const opened = owner.open(dialogWindowUrl, '_blank', `popup=yes,width=${options.width},height=${options.height},left=${left},top=${top}`)
  if (!opened) return null
  const popup: Window = opened
  const source = owner.document, doc = popup.document
  const base = doc.createElement('base')
  base.setAttribute('href', source.baseURI)
  doc.head.appendChild(base)
  doc.title = options.title
  const target = doc.createElement('div')
  target.className = 'detached-window-surface'
  doc.body.appendChild(target)
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
  syncStyles()
  const styles = new MutationObserver(syncStyles)
  styles.observe(source.head, { childList: true, subtree: true, characterData: true, attributes: true })
  styles.observe(source.documentElement, { attributes: true })
  let released = false
  const focused = source.activeElement as HTMLElement | null
  const key = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && !event.defaultPrevented && !event.isComposing && !event.repeat) {
      event.preventDefault(); escape()
    }
  }
  popup.addEventListener('keydown', key)
  // 系统关闭按钮与浏览器关闭都走同一回调；父窗口释放时同步关闭子窗口。
  const timer = owner.setInterval(() => { if (popup.closed && !released) { dispose(); closed() } }, 250)
  function dispose() {
    if (released) return
    released = true; owner.clearInterval(timer); styles.disconnect()
    popup.removeEventListener('keydown', key)
    owner.removeEventListener('pagehide', dispose)
    if (!popup.closed) popup.close()
    if (focused?.isConnected) focused.focus()
  }
  owner.addEventListener('pagehide', dispose)
  popup.focus()
  return { window: popup, target, dispose }
}
