import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import type { ClipboardItem as Item } from '../../src/features/clipboard/clipboard'

const api = vi.hoisted(() => ({ clipboardImage: vi.fn(), clipboardText: vi.fn(), clipboardThumbnail: vi.fn() }))
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

it.each(['image', 'text'] as const)('%s 双击和回车只预览，Ctrl+C 复制；连续单击不会重复切换选择', async kind => {
  const select = vi.fn(), preview = vi.fn(), restore = vi.fn()
  memoryRenderer.render(h(ClipboardItem, { item: { ...image, kind }, selected: true, busy: false, onSelect: select, onPreview: preview, onRestore: restore }), root)
  await flushComponent()
  const content = findNode(root, node => node.props.class === 'history-content')!
  content.props.onClick({ detail: 1, ctrlKey: false, metaKey: false, shiftKey: false })
  content.props.onClick({ detail: 2 })
  content.props.onDblclick()
  expect(select.mock.calls).toEqual([[{ toggle: false, range: false }]])
  expect(preview).toHaveBeenCalledOnce()
  expect(restore).not.toHaveBeenCalled()
  const article = findNode(root, node => node.type === 'article')!
  const event = { target: { matches: () => false }, repeat: false, isComposing: false, preventDefault: vi.fn(), stopPropagation: vi.fn() }
  article.props.onKeydown({ ...event, key: 'Enter' })
  expect(preview).toHaveBeenCalledTimes(2)
  article.props.onKeydown({ ...event, key: 'c', ctrlKey: true })
  expect(restore).toHaveBeenCalledOnce()
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
