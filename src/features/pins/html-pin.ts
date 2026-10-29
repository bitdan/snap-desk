/** 富文本只保留常用排版；不加载外链、不执行脚本、不继承剪贴板中的任意 CSS。 */
const tags = new Set('div span p br b strong i em u s del sub sup pre code blockquote h1 h2 h3 h4 h5 h6 ul ol li table thead tbody tfoot tr td th hr a font'.split(' '))
const blocked = new Set('script style iframe object embed svg math template link meta base input textarea select button audio video canvas noscript'.split(' '))
const colors = /^(#[\da-f]{3,8}|[a-z]{1,20}|rgba?\([\d.,% ]+\))$/i
const lengths = /^(0|\d{1,3}(\.\d{1,2})?(px|pt|em|rem|%))$/i

/** 样式值白名单独立于 DOM，便于验证 URL、表达式和布局逃逸不会进入渲染文档。 */
export function safeRichStyle(property: string, value: string): boolean {
  if (value.length > 80) return false
  if (['color', 'background-color', 'border-color'].includes(property)) return colors.test(value)
  if (['font-size', 'padding', 'padding-left', 'padding-right', 'padding-top', 'padding-bottom', 'margin-top', 'margin-bottom', 'border-width'].includes(property)) return lengths.test(value) && Number.parseFloat(value) <= 64
  const options: Record<string, string[]> = {
    'font-weight': ['normal', 'bold', '400', '500', '600', '700'], 'font-style': ['normal', 'italic'],
    'text-align': ['left', 'right', 'center', 'justify'], 'text-decoration': ['none', 'underline', 'line-through'],
    'white-space': ['normal', 'pre', 'pre-wrap', 'pre-line'], 'vertical-align': ['top', 'middle', 'bottom', 'baseline', 'sub', 'super'],
    'border-style': ['none', 'solid', 'dashed', 'dotted'],
  }
  return options[property]?.includes(value) ?? false
}

/** 使用 inert template 解析，复制允许的节点到新文档；原始 DOM 从不挂载。 */
export function richTextContent(html: string, owner: Document = document): HTMLElement {
  if (!html.trim() || new TextEncoder().encode(html).length > 1024 * 1024) throw new Error('富文本为空或超过 1 MiB。')
  const template = owner.createElement('template')
  template.innerHTML = html
  const root = owner.createElementNS('http://www.w3.org/1999/xhtml', 'div') as HTMLElement
  root.style.cssText = 'font:16px/1.5 Segoe UI,Microsoft YaHei,sans-serif;color:#24292f;background:#fff;padding:16px;box-sizing:border-box;overflow-wrap:anywhere;'
  let count = 0
  const copy = (source: Node, destination: Node, depth: number) => {
    if (++count > 4096 || depth > 40) throw new Error('富文本结构过于复杂，请缩小选区。')
    if (source.nodeType === 3) { destination.appendChild(owner.createTextNode(source.textContent ?? '')); return }
    if (source.nodeType !== 1) return
    const element = source as HTMLElement
    const tag = element.localName.toLowerCase()
    if (blocked.has(tag)) return
    // 图片不加载任何地址；保留替代文本。图片剪贴板仍由原有图片入口优先处理。
    if (tag === 'img') { if (element.getAttribute('alt')) destination.appendChild(owner.createTextNode(element.getAttribute('alt')!)); return }
    if (!tags.has(tag)) { for (const child of Array.from(source.childNodes)) copy(child, destination, depth + 1); return }
    const target = owner.createElementNS('http://www.w3.org/1999/xhtml', tag === 'font' ? 'span' : tag) as HTMLElement
    for (let index = 0; index < element.style.length; index++) {
      const property = element.style[index]
      const value = element.style.getPropertyValue(property).trim().toLowerCase()
      if (safeRichStyle(property, value)) target.style.setProperty(property, value)
    }
    const color = element.getAttribute('color')
    if (tag === 'font' && color && colors.test(color)) target.style.color = color
    if (tag === 'a') { target.style.color ||= '#2563eb'; target.style.textDecoration = 'underline' }
    if (tag === 'table') { target.style.borderCollapse = 'collapse'; target.style.maxWidth = '100%' }
    if (tag === 'td' || tag === 'th') {
      target.style.border = '1px solid #d1d5db'; target.style.padding ||= '4px'
      for (const name of ['colspan', 'rowspan']) {
        const value = Number(element.getAttribute(name))
        if (Number.isInteger(value) && value > 0 && value <= 32) target.setAttribute(name, String(value))
      }
    }
    destination.appendChild(target)
    for (const child of Array.from(source.childNodes)) copy(child, target, depth + 1)
  }
  for (const child of Array.from(template.content.childNodes)) copy(child, root, 0)
  if (!root.textContent?.trim()) throw new Error('没有可贴图的富文本。')
  return root
}

/** 在隐藏的测量容器中确定高度，再用无外部资源的 XHTML/SVG 转成 PNG。 */
export async function renderHtmlPin(html: string): Promise<string> {
  const content = richTextContent(html)
  const host = document.createElement('div')
  host.style.cssText = 'position:fixed;left:-100000px;top:0;width:640px;visibility:hidden;pointer-events:none;contain:layout style paint;'
  host.appendChild(content)
  document.body.appendChild(host)
  try {
    const width = 640
    const height = Math.ceil(content.getBoundingClientRect().height)
    if (height < 1 || height > 4096 || content.scrollWidth > width) throw new Error('富文本尺寸过大，请缩小选区。')
    const markup = new XMLSerializer().serializeToString(content)
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><foreignObject width="100%" height="100%">${markup}</foreignObject></svg>`
    const image = new Image()
    await new Promise<void>((resolve, reject) => {
      const timer = window.setTimeout(() => reject(new Error('富文本排版超时。')), 4000)
      image.onload = () => { clearTimeout(timer); resolve() }
      image.onerror = () => { clearTimeout(timer); reject(new Error('富文本排版失败。')) }
      image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
    })
    const canvas = document.createElement('canvas')
    canvas.width = width; canvas.height = height
    const context = canvas.getContext('2d')
    if (!context) throw new Error('图片画布不可用。')
    context.drawImage(image, 0, 0)
    return canvas.toDataURL('image/png')
  } finally { host.remove() }
}
