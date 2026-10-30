import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import ConfirmDialog from '../../src/shared/ui/ConfirmDialog.vue'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { handleWindowEscape } from '../../src/shared/ui/escape'

const api = vi.hoisted(() => ({ clipboardList: vi.fn(), clipboardStatus: vi.fn(), status: vi.fn(), clipboardPin: vi.fn(), clipboardClear: vi.fn(), clipboardDelete: vi.fn() }))
const clipboardEvents = vi.hoisted(() => ({ changed: () => {} }))
vi.mock('../../src/services/api', () => ({ api, desktop: true,
  onClipboardChanged: async (callback: () => void) => { clipboardEvents.changed = callback; return () => { clipboardEvents.changed = () => {} } }, onClipboardError: async () => () => {},
  onClipboardState: async () => () => {}, onClipboardOpen: async () => () => {},
}))
vi.mock('reka-ui', async () => {
  const { defineComponent, h } = await import('vue')
  const pass = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
  const hidden = defineComponent({ setup: () => () => null })
  return { DialogRoot: pass, DialogPortal: hidden, DialogOverlay: hidden, DialogContent: hidden,
    DialogTitle: pass, DialogDescription: pass, DialogTrigger: pass,
    AlertDialogRoot: pass, AlertDialogPortal: pass, AlertDialogOverlay: hidden, AlertDialogContent: pass,
    AlertDialogTitle: pass, AlertDialogDescription: pass, AlertDialogCancel: pass,
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
clientComponent(ConfirmDialog, new URL('../../src/shared/ui/ConfirmDialog.vue', import.meta.url))
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
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers() })

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

it('剪贴板变更事件静默刷新，只有点击加载更多才显示转圈和禁用按钮', async () => {
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  memoryRenderer.render(null, root)
  const page: { items: { id: number; kind: string }[]; nextCursor: string | null } = { items: [{ id: 1, kind: 'text' }], nextCursor: 'next' }
  api.clipboardList.mockResolvedValueOnce(page)
  memoryRenderer.render(h(Component), root); await flushComponent()
  vi.useFakeTimers()
  let complete!: (value: typeof page) => void
  api.clipboardList.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  clipboardEvents.changed()
  await vi.advanceTimersByTimeAsync(100)
  expect(findNode(root, node => node.props['aria-label'] === '加载更多')!.props.disabled).toBe(false)
  expect(findNode(root, node => node.type === 'button' && text(node) === '刷新')!.props.disabled).toBe(false)
  expect(findNode(root, node => node.props['aria-label'] === '加载中…')).toBeUndefined()
  expect(findNode(root, node => node.props['data-id'] === 1)).toBeDefined()
  complete(page); await flushComponent()
  api.clipboardList.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  findNode(root, node => node.props['aria-label'] === '加载更多')!.props.onClick()
  await flushComponent()
  expect(findNode(root, node => node.props['aria-label'] === '加载中…')!.props.disabled).toBe(true)
  complete({ ...page, nextCursor: null }); await flushComponent()
})

it.each([false, true])('清空通过确认框提交，删除收藏选项为 %s', async includeFavorites => {
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  findNode(root, node => node.type === 'button' && text(node) === '清空')!.props.onClick()
  await flushComponent()
  expect(api.clipboardClear).not.toHaveBeenCalled()
  const dialog = findNode(root, node => node.props.role === 'alertdialog')!
  const checkbox = findNode(dialog, node => node.type === 'input' && node.props.type === 'checkbox')!
  expect(checkbox.props['onUpdate:modelValue']).toBeTypeOf('function')
  checkbox.props['onUpdate:modelValue'](includeFavorites)
  findNode(dialog, node => node.type === 'button' && text(node) === '确认删除')!.props.onClick()
  await flushComponent()
  expect(api.clipboardClear).toHaveBeenCalledExactlyOnceWith(includeFavorites)
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeUndefined()
})

it('删除失败保留确认框和错误，忙碌期间阻止关闭与重复提交', async () => {
  const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
  await select(1, 2)
  findNode(root, node => node.props['aria-label'] === '删除所选条目')!.props.onClick()
  await flushComponent()
  const dialog = findNode(root, node => node.props.role === 'alertdialog')!
  let reject!: (error: Error) => void
  api.clipboardDelete.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
  const remove = findNode(dialog, node => node.type === 'button' && text(node) === '确认删除')!
  remove.props.onClick(); await flushComponent()
  expect(remove.props.disabled).toBe(true)
  remove.props.onClick()
  const event = { preventDefault: vi.fn() }
  dialog.props.onEscapeKeyDown(event)
  expect(event.preventDefault).toHaveBeenCalledOnce()
  expect(api.clipboardDelete).toHaveBeenCalledExactlyOnceWith([1, 2])
  reject(new Error('删除失败')); await flushComponent()
  expect(findNode(dialog, node => node.props.role === 'alert')?.text).toContain('删除失败')
  expect(remove.props.disabled).toBe(false)
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeDefined()
})
