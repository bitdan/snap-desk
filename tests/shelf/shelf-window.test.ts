import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultShelfSettings, type ShelfSnapshot } from '../../src/features/shelf/types'

const api = vi.hoisted(() => ({ shelfSnapshot: vi.fn(), shelfVisibility: vi.fn(), shelfItemAction: vi.fn(), clipboardRestore: vi.fn(), clipboardDrag: vi.fn(), clipboardPin: vi.fn(), clipboardThumbnail: vi.fn() }))
const events = vi.hoisted(() => ({ changed: () => {}, revealed: () => {}, capture: (_value: boolean) => {}, off: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, onShelfChanged: vi.fn(async (callback: () => void) => { events.changed = callback; return events.off }), onShelfRevealed: vi.fn(async (callback: () => void) => { events.revealed = callback; return events.off }), onShelfCaptureState: vi.fn(async (callback: (suspended: boolean) => void) => { events.capture = callback; return events.off }) }))
import ShelfCard from '../../src/features/shelf/components/ShelfCard.vue'
import ShelfWindow from '../../src/features/shelf/components/ShelfWindow.vue'
clientComponent(ShelfCard, new URL('../../src/features/shelf/components/ShelfCard.vue', import.meta.url))
const Component = clientComponent(ShelfWindow, new URL('../../src/features/shelf/components/ShelfWindow.vue', import.meta.url))
let root: MemoryNode, snapshot: ShelfSnapshot
const button = (label: string) => findNode(root, n => n.type === 'button' && n.props['aria-label'] === label)!

beforeEach(async () => {
  vi.resetAllMocks(); vi.useFakeTimers(); vi.stubGlobal('window', new EventTarget())
  snapshot = { settings: { ...defaultShelfSettings(), enabled: true }, keptIds: [7], recordingEnabled: true, items: [{ id: 7, kind: 'image', source: 'capture', preview: '截图', mimeType: 'image/png', width: 20, height: 20, byteSize: 200, favorite: false, createdAt: 1, lastSeenAt: 1, seenCount: 1 }] }
  api.shelfSnapshot.mockImplementation(async () => structuredClone(snapshot))
  api.clipboardThumbnail.mockResolvedValue('data:image/webp;base64,a')
  api.shelfVisibility.mockResolvedValue(undefined)
  api.shelfItemAction.mockResolvedValue(undefined)
  root = new MemoryNode('root'); memoryRenderer.render(h(Component), root); await flushComponent()
})
afterEach(() => { memoryRenderer.render(null, root); vi.useRealTimers(); vi.unstubAllGlobals() })

it('卡片复制沿用历史 ID；历史删除事件立即移除卡片', async () => {
  button('复制').props.onClick(); await flushComponent()
  expect(api.clipboardRestore).toHaveBeenCalledWith(7)
  snapshot.items = []; events.changed(); await flushComponent()
  expect(findNode(root, n => n.type === 'article')).toBeUndefined()
  expect(api.shelfItemAction).not.toHaveBeenCalled()
})

it('贴图创建失败不取下卡片；成功后才从架子移除', async () => {
  api.clipboardPin.mockRejectedValueOnce(new Error('贴图数量已达上限'))
  button('取下为贴图').props.onClick(); await flushComponent()
  expect(api.shelfItemAction).not.toHaveBeenCalled()
  expect(findNode(root, n => n.type === 'article')).toBeDefined()
  button('取下为贴图').props.onClick(); await flushComponent()
  expect(api.clipboardPin).toHaveBeenLastCalledWith(7)
  expect(api.shelfItemAction).toHaveBeenLastCalledWith(7, 'remove')
})

it('从架子移除后可撤销，并恢复原固定状态', async () => {
  button('从架子移除，保留历史').props.onClick(); await flushComponent()
  expect(api.shelfItemAction).toHaveBeenCalledWith(7, 'remove')
  button('撤销移除').props.onClick(); await flushComponent()
  expect(api.shelfItemAction).toHaveBeenLastCalledWith(7, 'keep')
})

it('指针在架上时不收起；离开后延迟隐藏原生窗口', async () => {
  const main = findNode(root, n => n.type === 'main')!
  main.props.onPointerenter(); await vi.advanceTimersByTimeAsync(800)
  expect(api.shelfVisibility).not.toHaveBeenCalled()
  main.props.onPointerleave(); await vi.advanceTimersByTimeAsync(699)
  expect(api.shelfVisibility).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(1)
  expect(api.shelfVisibility).toHaveBeenCalledWith(false)
})

it('通过翻页访问所有条目，删除最后一组后回到有效页', async () => {
  const first = snapshot.items[0]
  snapshot.items = Array.from({ length: 6 }, (_, index) => ({ ...first, id: index + 7, preview: `截图${index}` }))
  events.changed(); await flushComponent()
  expect(button('上一组').props.disabled).toBe(true)
  button('下一组').props.onClick(); await flushComponent()
  // 测试宿主默认宽度 900，每组 4 项；最后两项均可访问和操作。
  expect(button('复制：截图4')).toBeDefined()
  expect(button('复制：截图5')).toBeDefined()
  expect(button('下一组').props.disabled).toBe(true)
  events.revealed(); await flushComponent()
  expect(button('复制：截图0')).toBeDefined()
  button('下一组').props.onClick(); await flushComponent()
  snapshot.items = snapshot.items.slice(0, 2); events.changed(); await flushComponent()
  expect(button('复制：截图0')).toBeDefined()
  expect(findNode(root, n => n.props['aria-label'] === '暂存架翻页')).toBeUndefined()
})

it('截图期间取消旧收起计时器，历史刷新不收起；恢复后保留查看时间', async () => {
  const main = findNode(root, n => n.type === 'main')!
  main.props.onPointerleave()
  await vi.advanceTimersByTimeAsync(500)
  events.capture(true)
  events.changed(); await flushComponent()
  await vi.advanceTimersByTimeAsync(5000)
  expect(api.shelfVisibility).not.toHaveBeenCalled()
  events.capture(false)
  events.changed(); await flushComponent()
  await vi.advanceTimersByTimeAsync(1799)
  expect(api.shelfVisibility).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(1)
  expect(api.shelfVisibility).toHaveBeenCalledWith(false)
})

it('恢复后进入卡片取消自动收起，再离开使用普通延时', async () => {
  events.capture(true); events.capture(false)
  const main = findNode(root, n => n.type === 'main')!
  main.props.onPointerenter()
  await vi.advanceTimersByTimeAsync(2500)
  expect(api.shelfVisibility).not.toHaveBeenCalled()
  main.props.onPointerleave(); await vi.advanceTimersByTimeAsync(700)
  expect(api.shelfVisibility).toHaveBeenCalledWith(false)
})
