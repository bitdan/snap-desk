vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import ConfirmDialog from '../../src/shared/ui/ConfirmDialog.vue'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { handleWindowEscape } from '../../src/shared/ui/escape'

const api = vi.hoisted(() => ({ clipboardList: vi.fn(), clipboardStatus: vi.fn(), status: vi.fn(), clipboardPin: vi.fn(), clipboardClear: vi.fn(), clipboardDelete: vi.fn() }))
const clipboardEvents = vi.hoisted(() => ({ changed: () => {}, error: (_value: string) => {} }))
const listControls = vi.hoisted(() => ({ resetScroll: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true,
  onClipboardChanged: async (callback: () => void) => { clipboardEvents.changed = callback; return () => { clipboardEvents.changed = () => {} } },
  onClipboardError: async (callback: (value: string) => void) => { clipboardEvents.error = callback; return () => { clipboardEvents.error = () => {} } },
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
  return { default: defineComponent({ props: ['items', 'layout'], setup: (props, { slots, expose }) => {
    expose(listControls)
    return () => h('div', { 'data-layout': props.layout }, [...props.items.map((item: unknown) => slots.default?.({ item })), slots.footer?.()])
  } }) }
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
let savedPreferences: Map<string, string>
const pinButton = () => findNode(root, node => node.props['aria-label'] === '将所选内容贴图')!
const nodeText = (node: MemoryNode): string => node.text + node.children.map(nodeText).join('')
const refreshButton = () => findNode(root, node => node.type === 'button' && nodeText(node) === '刷新')!
const updatedNotice = () => findNode(root, node => node.props.role === 'status' && nodeText(node) === '内容已更新，点击刷新。')
async function select(...ids: number[]) {
  for (const id of ids) { findNode(root, node => node.props['data-id'] === id)!.props.onClick(); await flushComponent() }
}
beforeEach(async () => {
  vi.resetAllMocks()
  vi.stubGlobal('window', new EventTarget())
  savedPreferences = new Map()
  vi.stubGlobal('localStorage', { getItem: (key: string) => savedPreferences.get(key) ?? null, setItem: (key: string, value: string) => savedPreferences.set(key, value) })
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
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
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers(); vi.unstubAllGlobals() })

const categoryButtons = () => findNode(root, node => node.props.class === 'history-categories')!.children.filter(node => node.props['data-clipboard-category'] !== undefined)
const categoryKeys = () => categoryButtons().map(node => node.props['data-clipboard-category'])
const categoryTab = (key: string) => categoryButtons().find(node => node.props['data-clipboard-category'] === key)!

it('默认分类顺序为全部、文本、图片、文件、链接、文件路径、颜色、收藏', () => {
  expect(categoryKeys()).toEqual(['', 'text', 'image', 'file', 'link', 'path', 'color', 'favorites'])
})

it('拖拽调整分类顺序并持久保存，拖动产生的点击不切换分类', async () => {
  const parent = {}, source = { parentElement: parent, dataset: { clipboardCategory: 'link' }, setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() }
  const destination = { parentElement: parent, dataset: { clipboardCategory: 'image' } }
  vi.stubGlobal('document', { elementFromPoint: () => ({ closest: () => destination }) })
  const button = categoryTab('link')
  const event = { button: 0, pointerId: 1, clientX: 10, clientY: 10, currentTarget: source }
  api.clipboardList.mockClear()
  button.props.onPointerdown(event)
  button.props.onPointermove({ ...event, clientX: 100 })
  button.props.onPointerup({ ...event, clientX: 100 })
  button.props.onClick({ detail: 1 }); await flushComponent()
  const expected = ['', 'text', 'link', 'image', 'file', 'path', 'color', 'favorites']
  expect(categoryKeys()).toEqual(expected)
  expect(categoryTab('').props['aria-pressed']).toBe(true)
  expect(api.clipboardList).not.toHaveBeenCalled()
  expect(source.releasePointerCapture).toHaveBeenCalledWith(1)
  expect(JSON.parse(savedPreferences.get('snap-desk.clipboard-category-order.v1')!)).toEqual(expected)
  memoryRenderer.render(null, root)
  memoryRenderer.render(h(Component), root); await flushComponent()
  expect(categoryKeys()).toEqual(expected)
})

it('拖到分类栏外不改顺序，取消拖动后仍可以用键盘选择', async () => {
  const source = { parentElement: {}, setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() }
  vi.stubGlobal('document', { elementFromPoint: () => null })
  const button = categoryTab('link'), before = categoryKeys()
  const event = { button: 0, pointerId: 1, clientX: 10, clientY: 10, currentTarget: source }
  button.props.onPointerdown(event)
  button.props.onPointerup({ ...event, clientX: 100 }); await flushComponent()
  expect(categoryKeys()).toEqual(before)
  button.props.onClick({ detail: 0 }); await flushComponent()
  expect(categoryTab('link').props['aria-pressed']).toBe(true)
})

it('Alt 加方向键排序，损坏或过时存储自动补齐分类', async () => {
  const event = { altKey: true, key: 'ArrowLeft', preventDefault: vi.fn(), stopPropagation: vi.fn(), currentTarget: { focus: vi.fn() } }
  categoryTab('image').props.onKeydown(event); await flushComponent()
  expect(categoryKeys().slice(0, 3)).toEqual(['', 'image', 'text'])
  expect(event.currentTarget.focus).toHaveBeenCalled()
  savedPreferences.set('snap-desk.clipboard-category-order.v1', '["link","link","unknown"]')
  memoryRenderer.render(null, root)
  memoryRenderer.render(h(Component), root); await flushComponent()
  expect(categoryKeys()).toEqual(['link', '', 'text', 'image', 'file', 'path', 'color', 'favorites'])
})

it.each([
  ['链接', 'text', 'link'], ['颜色', 'text', 'color'], ['文件路径', 'text', 'path'],
  ['文件', 'file', undefined], ['图片', 'image', undefined], ['文本', 'text', undefined],
] as const)('%s 分类向后端发送筛选条件，切回全部清除分类和游标', async (label, kind, textCategory) => {
  const tab = (label: string) => findNode(root, node => node.type === 'button' && nodeText(node) === label)!
  await select(1)
  api.clipboardList.mockClear()
  tab(label).props.onClick({ detail: 0 }); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ kind, textCategory, cursor: undefined }))
  expect(pinButton()).toBeUndefined()
  tab('全部').props.onClick({ detail: 0 }); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ kind: undefined, textCategory: undefined, cursor: undefined }))
})

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
  await select(1)
  findNode(root, node => node.props['aria-label'] === '组合导出所选条目')!.props.onClick()
  await flushComponent()
  const dialog = () => findNode(root, node => node.props['data-window'] === '组合导出')
  expect(dialog()).toBeDefined()
  handleWindowEscape(key(), exit)
  await flushComponent()
  expect(dialog()).toBeUndefined()
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
it('文字、图片和文件引用均可创建贴图，文件仅展示路径', async () => {
  await select(4, 1, 3)
  expect(pinButton().props.disabled).toBe(false)
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin.mock.calls).toEqual([[4], [1], [3]])
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

it('加载中搜索与剪贴板事件交错时仍按输入查询，并忽略迟到的旧结果', async () => {
  vi.useFakeTimers()
  let complete!: (value: { items: { id: number; kind: string }[]; nextCursor: null }) => void
  api.clipboardList.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  const refreshing = refreshButton().props.onClick()
  await flushComponent()
  findNode(root, node => node.props['aria-label'] === '搜索文本或文件路径')!.props['onUpdate:modelValue']('报告')
  await flushComponent()
  clipboardEvents.changed()
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 9, kind: 'text' }], nextCursor: null })
  await vi.advanceTimersByTimeAsync(250); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ keyword: '报告', cursor: undefined }))
  expect(findNode(root, node => node.props['data-id'] === 9)).toBeDefined()
  expect(refreshButton().props.disabled).toBe(false)
  complete({ items: [{ id: 8, kind: 'text' }], nextCursor: null })
  await refreshing; await flushComponent()
  expect(findNode(root, node => node.props['data-id'] === 9)).toBeDefined()
  expect(findNode(root, node => node.props['data-id'] === 8)).toBeUndefined()
  expect(updatedNotice()).toBeUndefined()
})

it('浏览后续分页时后台变更保留列表和游标，主动刷新后移除不可见选择', async () => {
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 1, kind: 'text' }], nextCursor: 'page-2' })
  await refreshButton().props.onClick(); await flushComponent()
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 5, kind: 'image' }], nextCursor: 'page-3' })
  await findNode(root, node => node.props['aria-label'] === '加载更多')!.props.onClick(); await flushComponent()
  vi.useFakeTimers()
  const requests = api.clipboardList.mock.calls.length
  clipboardEvents.changed()
  await vi.advanceTimersByTimeAsync(500); await flushComponent()
  expect(api.clipboardList).toHaveBeenCalledTimes(requests)
  expect(findNode(root, node => node.props['data-id'] === 1)).toBeDefined()
  expect(findNode(root, node => node.props['data-id'] === 5)).toBeDefined()
  expect(updatedNotice()).toBeDefined()
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 6, kind: 'text' }], nextCursor: null })
  await findNode(root, node => node.props['aria-label'] === '加载更多')!.props.onClick(); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'page-3' }))
  expect(updatedNotice()).toBeDefined()
  await select(5)
  api.clipboardList.mockResolvedValueOnce({ items: [{ id: 9, kind: 'text' }], nextCursor: null })
  await refreshButton().props.onClick(); await flushComponent()
  expect(api.clipboardList).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: undefined }))
  expect(findNode(root, node => node.props['data-id'] === 5)).toBeUndefined()
  expect(pinButton()).toBeUndefined()
  expect(updatedNotice()).toBeUndefined()
  expect(listControls.resetScroll).toHaveBeenCalled()
})

it('后台查询在途时开始选择，迟到结果保留当前记录与选择', async () => {
  vi.useFakeTimers()
  let complete!: (value: { items: { id: number; kind: string }[]; nextCursor: null }) => void
  api.clipboardList.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  clipboardEvents.changed()
  await vi.advanceTimersByTimeAsync(100)
  await select(2)
  complete({ items: [{ id: 9, kind: 'text' }], nextCursor: null }); await flushComponent()
  expect(findNode(root, node => node.props['data-id'] === 2)).toBeDefined()
  expect(findNode(root, node => node.props['data-id'] === 9)).toBeUndefined()
  expect(updatedNotice()).toBeDefined()
  await pinButton().props.onClick(); await flushComponent()
  expect(api.clipboardPin).toHaveBeenCalledExactlyOnceWith(2)
})

it('列表向下滚动后只提示更新，不自动替换正在浏览的记录', async () => {
  vi.useFakeTimers()
  findNode(root, node => node.props['aria-label'] === '历史记录列表')!.props.onScroll({ target: { scrollTop: 100 } })
  const requests = api.clipboardList.mock.calls.length
  clipboardEvents.changed()
  await vi.advanceTimersByTimeAsync(500); await flushComponent()
  expect(api.clipboardList).toHaveBeenCalledTimes(requests)
  expect(findNode(root, node => node.props['data-id'] === 1)).toBeDefined()
  expect(updatedNotice()).toBeDefined()
})

it('剪贴板恢复事件同时清除提示与状态快照中的旧错误', async () => {
  memoryRenderer.render(null, root)
  api.clipboardStatus.mockResolvedValueOnce({ count: 4, byteSize: 0, supported: true, enabled: true, error: '剪贴板暂时不可用' })
  memoryRenderer.render(h(Component), root); await flushComponent()
  clipboardEvents.error('剪贴板暂时不可用'); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')?.text).toContain('剪贴板暂时不可用')
  clipboardEvents.error(''); await flushComponent()
  expect(findNode(root, node => node.props.role === 'alert')).toBeUndefined()
})

it.each(['', '维护故障仍未恢复'])('列表在途时错误事件 %j 优先于迟到的状态快照', async currentError => {
  let complete!: (value: { items: { id: number; kind: string }[]; nextCursor: null }) => void
  api.clipboardList.mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
  api.clipboardStatus.mockResolvedValueOnce({ count: 1, byteSize: 0, supported: true, enabled: true, error: '旧的单次采集故障' })
  const refreshing = refreshButton().props.onClick()
  await flushComponent()
  clipboardEvents.error(currentError); await flushComponent()
  complete({ items: [{ id: 1, kind: 'text' }], nextCursor: null })
  await refreshing; await flushComponent()
  // 后续正常操作会清除操作提示；状态快照仍须保留最新持续故障或恢复状态。
  await select(1)
  await pinButton().props.onClick(); await flushComponent()
  const alert = findNode(root, node => node.props.role === 'alert')
  if (currentError) expect(alert?.text).toBe(currentError)
  else expect(alert).toBeUndefined()
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
  const event = { key: 'Escape', defaultPrevented: false, isComposing: false, repeat: false, preventDefault: vi.fn(), stopPropagation: vi.fn() }
  dialog.props.onKeydown(event)
  expect(event.preventDefault).toHaveBeenCalledOnce()
  await flushComponent()
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeDefined()
  expect(api.clipboardDelete).toHaveBeenCalledExactlyOnceWith([1, 2])
  reject(new Error('删除失败')); await flushComponent()
  expect(findNode(dialog, node => node.props.role === 'alert')?.text).toContain('删除失败')
  expect(remove.props.disabled).toBe(false)
  expect(findNode(root, node => node.props.role === 'alertdialog')).toBeDefined()
})
