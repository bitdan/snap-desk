import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { handleWindowEscape } from '../../src/shared/ui/escape'

const api = vi.hoisted(() => ({ clipboardList: vi.fn(), clipboardStatus: vi.fn(), status: vi.fn(), clipboardPin: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true,
  onClipboardChanged: async () => () => {}, onClipboardError: async () => () => {},
  onClipboardState: async () => () => {}, onClipboardOpen: async () => () => {},
}))
vi.mock('reka-ui', async () => {
  const { defineComponent, h } = await import('vue')
  const pass = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
  const hidden = defineComponent({ setup: () => () => null })
  return { DialogRoot: pass, DialogPortal: hidden, DialogOverlay: hidden, DialogContent: hidden,
    DialogTitle: pass, DialogDescription: pass, DialogTrigger: pass,
    PopoverRoot: pass, PopoverTrigger: pass, PopoverPortal: hidden, PopoverContent: hidden }
})
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/clipboard/components/VirtualClipboardList.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ props: ['items', 'layout'], setup: (props, { slots }) => () => h('div', { 'data-layout': props.layout }, [...props.items.map((item: unknown) => slots.default?.({ item })), slots.footer?.()]) }) }
})
vi.mock('../../src/features/clipboard/components/ClipboardPreview.vue', async () => {
  const { defineComponent } = await import('vue')
  return { default: defineComponent({ setup: () => () => null }) }
})
vi.mock('../../src/features/clipboard/components/ClipboardItem.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ props: ['item'], emits: ['select'], setup: (props, { emit }) => () => h('button', { 'data-id': props.item.id, onClick: () => emit('select') }) }) }
})
import History from '../../src/features/clipboard/components/ClipboardHistory.vue'
const Component = clientComponent(History, new URL('../../src/features/clipboard/components/ClipboardHistory.vue', import.meta.url))
let root: MemoryNode
const pinButton = () => findNode(root, node => node.props['aria-label'] === '将所选文字和图片贴图')!
async function select(...ids: number[]) {
  for (const id of ids) { findNode(root, node => node.props['data-id'] === id)!.props.onClick(); await flushComponent() }
}
beforeEach(async () => {
  vi.resetAllMocks()
  Object.assign(MemoryNode.prototype, { select: vi.fn() })
  root = new MemoryNode('root')
  api.status.mockResolvedValue({ settings: { clipboard: { enabled: true } } })
  api.clipboardStatus.mockResolvedValue({ count: 4, byteSize: 0, supported: true, enabled: true })
  api.clipboardList.mockResolvedValue({ items: [
    { id: 1, kind: 'text' }, { id: 2, kind: 'text' }, { id: 3, kind: 'image' }, { id: 4, kind: 'file' },
  ], nextCursor: null })
  api.clipboardPin.mockResolvedValue(undefined)
  memoryRenderer.render(h(Component), root); await flushComponent()
})
afterEach(() => { memoryRenderer.render(null, root) })

it('清空确认和导出弹窗先响应 Esc，焦点不在弹窗内也不直接退出主窗口', async () => {
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  const key = () => ({ key: 'Escape', defaultPrevented: false, isComposing: false, repeat: false, preventDefault: vi.fn() }) as unknown as KeyboardEvent
  const exit = vi.fn()
  findNode(root, node => node.type === 'button' && text(node) === '清空')!.props.onClick()
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeDefined()
  handleWindowEscape(key(), exit)
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeUndefined()
  expect(exit).not.toHaveBeenCalled()
  const dialog = () => findNode(root, node => typeof node.props['onUpdate:open'] === 'function')!
  dialog().props['onUpdate:open'](true)
  await flushComponent()
  expect(dialog().props.open).toBe(true)
  handleWindowEscape(key(), exit)
  await flushComponent()
  expect(dialog().props.open).toBe(false)
  expect(exit).not.toHaveBeenCalled()
})

it('没有选择或关键词时，剪贴板页不吞掉主窗口 Esc', () => {
  const event = { key: 'Escape', defaultPrevented: false, isComposing: false, target: new MemoryNode('input'), preventDefault: vi.fn() }
  findNode(root, node => node.props.class === 'clipboard-history')!.props.onKeydown(event)
  expect(event.preventDefault).not.toHaveBeenCalled()
})

it('列表和网格可切换，保留已选条目及批量操作顺序', async () => {
  await select(2, 1)
  findNode(root, node => node.props['aria-label'] === '网格视图')!.props.onClick()
  await flushComponent()
  expect(findNode(root, node => node.props['data-layout'] === 'grid')).toBeDefined()
  expect(findNode(root, node => node.props['aria-label'] === '网格视图')!.props['aria-pressed']).toBe(true)
  findNode(root, node => node.props['aria-label'] === '列表视图')!.props.onClick()
  await flushComponent()
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin.mock.calls).toEqual([[2], [1]])
})

it('底部省略号加载下一页，保留原有记录并提交当前游标', async () => {
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 1, kind: 'text' }], nextCursor: 'page-2' })
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  await findNode(root, node => node.type === 'button' && text(node) === '刷新')!.props.onClick()
  await flushComponent()
  const more = findNode(root, node => node.props['aria-label'] === '加载更多')!
  expect(more.parent?.props.class).toBe('history-load-more')
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 5, kind: 'image' }], nextCursor: null })
  await more.props.onClick(); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'page-2' }))
  expect(findNode(root, node => node.props['data-id'] === 1)).toBeDefined()
  expect(findNode(root, node => node.props['data-id'] === 5)).toBeDefined()
  expect(findNode(root, node => node.props['aria-label'] === '加载更多')).toBeUndefined()
})

it('多条文字可直接贴图，按选择顺序调用历史 ID，不读取预览或改写剪贴板', async () => {
  await select(2, 1)
  expect(pinButton().props.disabled).toBe(false)
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin.mock.calls).toEqual([[2], [1]])
})
it('文字与图片混选均创建贴图，文件引用不参与', async () => {
  await select(4)
  expect(pinButton().props.disabled).toBe(true)
  await select(1, 3)
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin.mock.calls).toEqual([[1], [3]])
})
it('中途失败停止创建，显示已完成数量，允许用户继续操作', async () => {
  api.clipboardPin.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('容量已满'))
  await select(2, 1, 3)
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin.mock.calls).toEqual([[2], [1]])
  expect(findNode(root, node => node.props.role === 'alert')?.text).toContain('已创建 1 张贴图')
  expect(pinButton().props.disabled).toBe(false)
})
