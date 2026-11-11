vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import type { ClipboardItem as Item } from '../../src/features/clipboard/clipboard'

const api = vi.hoisted(() => ({ clipboardImage: vi.fn(), clipboardText: vi.fn(), clipboardThumbnail: vi.fn(), clipboardOpenLink: vi.fn(), clipboardOpenPath: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('reka-ui', async () => {
  const { defineComponent, h } = await import('vue')
  const pass = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
  return Object.fromEntries(['ContextMenuRoot', 'ContextMenuTrigger', 'ContextMenuPortal', 'ContextMenuContent', 'ContextMenuItem', 'ContextMenuSeparator',
    'DialogRoot', 'DialogPortal', 'DialogOverlay', 'DialogContent', 'DialogTitle', 'DialogDescription'].map(name => [name, pass]))
})
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import ItemComponent from '../../src/features/clipboard/components/ClipboardItem.vue'
import PreviewComponent from '../../src/features/clipboard/components/ClipboardPreview.vue'
const ClipboardItem = clientComponent(ItemComponent, new URL('../../src/features/clipboard/components/ClipboardItem.vue', import.meta.url))
const ClipboardPreview = clientComponent(PreviewComponent, new URL('../../src/features/clipboard/components/ClipboardPreview.vue', import.meta.url))
const root = new MemoryNode('root')
const image = { id: 7, kind: 'image', width: 1200, height: 800, preview: '', lastSeenAt: 1, seenCount: 1 } as Item
beforeEach(() => {
  vi.resetAllMocks()
  vi.stubGlobal('IntersectionObserver', class { observe = vi.fn(); disconnect = vi.fn() })
})
afterEach(() => { memoryRenderer.render(null, root); vi.unstubAllGlobals() })

it.each(['image', 'text'] as const)('%s 双击和回车复制并收起，Ctrl+C 仅复制，预览走悬浮按钮；连续单击不会重复切换选择', async kind => {
  const select = vi.fn(), preview = vi.fn(), restore = vi.fn(), activate = vi.fn()
  memoryRenderer.render(h(ClipboardItem, { item: { ...image, kind }, selected: true, busy: false, onSelect: select, onPreview: preview, onRestore: restore, onActivate: activate }), root)
  await flushComponent()
  const article = findNode(root, node => node.type === 'article')!
  article.props.onClick({ detail: 1, ctrlKey: false, metaKey: false, shiftKey: false })
  article.props.onClick({ detail: 2 })
  article.props.onDblclick()
  expect(select.mock.calls).toEqual([[{ toggle: false, range: false }]])
  expect(activate).toHaveBeenCalledOnce()
  expect(preview).not.toHaveBeenCalled()
  const event = { target: { matches: () => false }, repeat: false, isComposing: false, preventDefault: vi.fn(), stopPropagation: vi.fn() }
  article.props.onKeydown({ ...event, key: 'Enter' })
  expect(activate).toHaveBeenCalledTimes(2)
  // 焦点在卡片内的操作按钮上时，回车属于按钮自身，不再触发整条复制。
  article.props.onKeydown({ ...event, key: 'Enter', target: { matches: () => true } })
  expect(activate).toHaveBeenCalledTimes(2)
  article.props.onKeydown({ ...event, key: 'c', ctrlKey: true })
  expect(restore).toHaveBeenCalledOnce()
  findNode(root, node => node.type === 'button' && node.props['aria-label'] === '预览')!.props.onClick()
  expect(preview).toHaveBeenCalledOnce()
  expect(restore).toHaveBeenCalledOnce()
})

it('暂存架未开启时不显示联动入口；开启后按架上状态切换放上与取下', async () => {
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  const entry = () => findNode(root, node => node.type === 'div' && ['放到暂存架', '从暂存架取下'].includes(text(node)))
  const shelf = vi.fn()
  memoryRenderer.render(h(ClipboardItem, { item: image, selected: false, busy: false, shelfKept: null, onShelf: shelf }), root)
  await flushComponent()
  expect(entry()).toBeUndefined()
  memoryRenderer.render(h(ClipboardItem, { item: image, selected: false, busy: false, shelfKept: false, onShelf: shelf }), root)
  await flushComponent()
  expect(text(entry()!)).toBe('放到暂存架')
  memoryRenderer.render(h(ClipboardItem, { item: image, selected: false, busy: false, shelfKept: true, onShelf: shelf }), root)
  await flushComponent()
  expect(text(entry()!)).toBe('从暂存架取下')
  entry()!.props.onSelect()
  expect(shelf).toHaveBeenCalledOnce()
})

it('文字预览读取全文并保留换行，允许选中复制，不用列表摘要冒充全文', async () => {
  const full = '完整第一行\n完整第二行\n' + '长文字'.repeat(1000)
  api.clipboardText.mockResolvedValue(full)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text', preview: '截断摘要' } }), root)
  await flushComponent()
  expect(api.clipboardText).toHaveBeenCalledExactlyOnceWith(7)
  expect(api.clipboardImage).not.toHaveBeenCalled()
  const text = findNode(root, node => node.type === 'textarea')!
  expect(text.props.value).toBe(full)
  expect(text.props.readonly).toBeDefined()
})

it.each([
  ['F:\\download\\ja-netfilter', 'F:\\download\\ja-netfilter'],
  ['"C:\\my files\\report.pdf"', 'C:\\my files\\report.pdf'],
  ['\\\\server\\share\\a.txt', '\\\\server\\share\\a.txt'],
  ['/home/user/a.txt', '/home/user/a.txt'],
])('文件路径显示可点击预览，定位时只提交历史 ID：%s', async (value, path) => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  const button = findNode(root, node => node.props['aria-label'] === '在资源管理器中打开路径')!
  expect(text(button)).toBe(path)
  expect(findNode(root, node => node.type === 'textarea')).toBeUndefined()
  const header = findNode(root, node => node.props.class === 'history-preview-header')!
  expect(findNode(root, node => node.props['data-window'] === '预览')).toBeDefined()
  expect(text(header)).not.toContain('预览')
  expect(text(header)).not.toContain('文字预览')
  let finish!: () => void
  api.clipboardOpenPath.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve }))
  button.props.onClick(); await flushComponent()
  button.props.onClick(); await flushComponent()
  expect(api.clipboardOpenPath).toHaveBeenCalledExactlyOnceWith(7)
  expect(api.clipboardOpenLink).not.toHaveBeenCalled()
  expect(button.props.disabled).toBe(true)
  finish(); await flushComponent()
  expect(button.props.disabled).toBe(false)
})

it.each(['C:relative.txt', 'relative/a.txt', 'C:\\a.txt\nother text', '\\\\.\\pipe\\name', '\\\\?\\C:\\a.txt'])('非绝对路径或设备路径仍作为普通文本：%s', async value => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  expect(findNode(root, node => node.props['aria-label'] === '在资源管理器中打开路径')).toBeUndefined()
  expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe(value)
})

it('路径不可访问时保留预览并展示错误，允许再次点击', async () => {
  api.clipboardText.mockResolvedValue('C:\\missing')
  api.clipboardOpenPath.mockRejectedValueOnce(new Error('路径不存在或无法访问。')).mockResolvedValueOnce(undefined)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  const button = findNode(root, node => node.props['aria-label'] === '在资源管理器中打开路径')!
  await button.props.onClick(); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')?.text).toContain('路径不存在或无法访问。')
  expect(button.props.disabled).toBe(false)
  await button.props.onClick(); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
})

it.each([
  ['#fff', '#fff'], [' #12AB3480 ', '#12ab3480'], ['#abcd', '#abcd'],
  ['rgb(255, 0, 12)', 'rgb(255, 0, 12)'], ['rgba(100%, 0%, 0%, 0)', 'rgba(255, 0, 0, 0)'],
  ['hsl(120, 50%, 30%)', 'hsl(120, 50%, 30%)'], ['hsla(240, 100%, 50%, 50%)', 'hsla(240, 100%, 50%, 0.5)'],
])('颜色全文渲染色块并保留原值供选择复制：%s', async (value, backgroundColor) => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text', preview: '摘要不用于解析' } }), root)
  await flushComponent()
  expect(api.clipboardText).toHaveBeenCalledExactlyOnceWith(7)
  expect(findNode(root, node => node.props.class === 'history-color-swatch')?.props.style).toEqual({ backgroundColor })
  expect(findNode(root, node => node.props.class === 'history-color-checkerboard')?.props.role).toBe('img')
  const input = findNode(root, node => node.props['aria-label'] === '颜色值')!
  expect(input.props.value).toBe(value)
  expect(input.props.readonly).toBeDefined()
  expect(findNode(root, node => node.type === 'textarea')).toBeUndefined()
})

it.each(['#12', '#ggg', 'rgb(256,0,0)', 'rgba(1,2,3,2)', 'hsl(1,2,3)', 'red', '#fff; background:url(test)', '第一行\n#fff'])('无效颜色或普通文字继续显示原文：%s', async value => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  expect(findNode(root, node => node.props.class === 'history-color-swatch')).toBeUndefined()
  expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe(value)
})

it('从颜色切换到普通文本时立即清除色块，等待新条目全文', async () => {
  let finish!: (value: string) => void
  api.clipboardText.mockResolvedValueOnce('#fff').mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, id: 8, kind: 'text' } }), root)
  await flushComponent()
  expect(findNode(root, node => node.props.class === 'history-color-swatch')).toBeUndefined()
  finish('普通文本'); await flushComponent()
  expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe('普通文本')
})

it.each(['https://example.com/a?q=中文', 'www.example.com/a'])('链接预览可点击打开，原生只接收记录 ID：%s', async value => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  const link = findNode(root, node => node.type === 'a')!
  expect(link.props.href).toBe(new URL(value.startsWith('www.') ? `https://${value}` : value).href)
  expect(findNode(root, node => node.type === 'textarea')).toBeUndefined()
  let finish!: () => void
  api.clipboardOpenLink.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve }))
  const event = { preventDefault: vi.fn() }
  link.props.onClick(event); await flushComponent()
  link.props.onClick(event); await flushComponent()
  expect(event.preventDefault).toHaveBeenCalled()
  expect(api.clipboardOpenLink).toHaveBeenCalledExactlyOnceWith(7)
  expect(link.props['aria-disabled']).toBe(true)
  finish(); await flushComponent()
  expect(link.props['aria-disabled']).toBe(false)
})

it.each(['javascript:alert(1)', 'file:///C:/a.txt', 'data:text/html,test', '请看 https://example.com', 'https://example.com\n其他文字'])('非链接或不支持的协议保持文字预览：%s', async value => {
  api.clipboardText.mockResolvedValue(value)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  expect(findNode(root, node => node.type === 'a')).toBeUndefined()
  expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe(value)
})

it('打开失败保留链接供重试', async () => {
  api.clipboardText.mockResolvedValue('https://example.com')
  api.clipboardOpenLink.mockRejectedValueOnce(new Error('open failed')).mockResolvedValueOnce(undefined)
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  const link = () => findNode(root, node => node.type === 'a')!
  await link().props.onClick({ preventDefault: vi.fn() }); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')?.text).toContain('open failed')
  await link().props.onClick({ preventDefault: vi.fn() }); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
  expect(api.clipboardOpenLink).toHaveBeenCalledTimes(2)
})

it('切换条目后丢弃上一条链接迟到的打开错误', async () => {
  let reject!: (error: Error) => void
  api.clipboardText.mockResolvedValue('https://example.com')
  api.clipboardOpenLink.mockImplementation(() => new Promise((_, fail) => { reject = fail }))
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind: 'text' } }), root)
  await flushComponent()
  findNode(root, node => node.type === 'a')!.props.onClick({ preventDefault: vi.fn() })
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, id: 8, kind: 'text' } }), root)
  await flushComponent()
  reject(new Error('old failure')); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
  expect(findNode(root, node => node.type === 'a')?.props['aria-disabled']).toBe(false)
})

it.each(['image', 'text'] as const)('%s 加载中按 Esc 退出并丢弃迟到内容，输入法组词不退出', async kind => {
  let finish!: (value: string) => void
  const close = vi.fn(), request = kind === 'image' ? api.clipboardImage : api.clipboardText
  request.mockImplementation(() => new Promise(resolve => { finish = resolve }))
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, kind }, onClose: close }), root)
  await flushComponent()
  const dialog = findNode(root, node => node.props.class === 'history-preview-dialog')!
  const event = { key: 'Escape', defaultPrevented: false, isComposing: false, repeat: false, preventDefault: vi.fn(), stopPropagation: vi.fn() }
  dialog.props.onKeydown({ ...event, isComposing: true })
  expect(close).not.toHaveBeenCalled()
  dialog.props.onKeydown(event)
  expect(event.preventDefault).toHaveBeenCalledOnce()
  expect(close).toHaveBeenCalledOnce()
  finish(kind === 'image' ? 'data:image/png;base64,late' : '迟到全文')
  await flushComponent()
  expect(findNode(root, node => node.type === 'img')).toBeUndefined()
  expect(findNode(root, node => node.type === 'textarea')?.props.value ?? '').toBe('')
})

it('预览请求只传 ID，关闭后迟到的原图不会重新驻留', async () => {
  let finish!: (value: string) => void
  api.clipboardImage.mockImplementation(() => new Promise(resolve => { finish = resolve }))
  memoryRenderer.render(h(ClipboardPreview, { item: image }), root)
  await flushComponent()
  expect(api.clipboardImage).toHaveBeenCalledExactlyOnceWith(7)
  memoryRenderer.render(h(ClipboardPreview, { item: null }), root)
  await flushComponent()
  finish('data:image/png;base64,old')
  await flushComponent()
  expect(findNode(root, node => node.type === 'img')).toBeUndefined()
})

it('预览读取失败可以重试，切换图片后不显示上一张原图', async () => {
  api.clipboardImage.mockRejectedValueOnce(new Error('missing image')).mockResolvedValueOnce('data:image/png;base64,first').mockResolvedValueOnce('data:image/png;base64,next')
  memoryRenderer.render(h(ClipboardPreview, { item: image }), root)
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeDefined()
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  await findNode(root, node => node.type === 'button' && text(node) === '重试')!.props.onClick()
  await flushComponent()
  expect(findNode(root, node => node.type === 'img')?.props.src).toBe('data:image/png;base64,first')
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, id: 8 } }), root)
  await flushComponent()
  expect(api.clipboardImage).toHaveBeenLastCalledWith(8)
  expect(findNode(root, node => node.type === 'img')?.props.src).toBe('data:image/png;base64,next')
})

it('旧图片迟到的解码错误不覆盖新图片或文本，当前图片解码失败仍可重试', async () => {
  api.clipboardImage.mockResolvedValueOnce('data:image/png;base64,first')
    .mockResolvedValueOnce('data:image/png;base64,next')
    .mockResolvedValueOnce('data:image/png;base64,retry')
  api.clipboardText.mockResolvedValue('新的文本')
  memoryRenderer.render(h(ClipboardPreview, { item: image }), root)
  await flushComponent()
  const oldImage = findNode(root, node => node.type === 'img')!
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, id: 8 } }), root)
  await flushComponent()
  oldImage.props.onError({ currentTarget: oldImage })
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
  const currentImage = findNode(root, node => node.type === 'img')!
  expect(currentImage.props.src).toBe('data:image/png;base64,next')
  currentImage.props.onError({ currentTarget: currentImage })
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeDefined()
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  await findNode(root, node => node.type === 'button' && text(node) === '重试')!.props.onClick()
  await flushComponent()
  expect(findNode(root, node => node.type === 'img')?.props.src).toBe('data:image/png;base64,retry')
  memoryRenderer.render(h(ClipboardPreview, { item: { ...image, id: 9, kind: 'text' } }), root)
  await flushComponent()
  currentImage.props.onError({ currentTarget: currentImage })
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
  expect(findNode(root, node => node.type === 'textarea')?.props.value).toBe('新的文本')
})
