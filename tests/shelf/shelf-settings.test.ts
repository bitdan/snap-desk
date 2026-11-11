import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultShelfSettings, shelfCategories } from '../../src/features/shelf/types'

const api = vi.hoisted(() => ({ shelfSnapshot: vi.fn(), shelfConfigure: vi.fn(), shelfVisibility: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api, desktop: true }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => ({ default: (await import('vue')).defineComponent({ setup: (_, { slots }) => () => h('button', slots.default?.()) }) }))
import ShelfSettings from '../../src/features/shelf/components/ShelfSettings.vue'
import InfoTip from '../../src/shared/ui/InfoTip.vue'
clientComponent(InfoTip, new URL('../../src/shared/ui/InfoTip.vue', import.meta.url))
const Component = clientComponent(ShelfSettings, new URL('../../src/features/shelf/components/ShelfSettings.vue', import.meta.url))
let root: MemoryNode
beforeEach(async () => {
  vi.resetAllMocks()
  api.shelfSnapshot.mockResolvedValue({ settings: defaultShelfSettings(), items: [], keptIds: [], recordingEnabled: true })
  api.shelfConfigure.mockResolvedValue(undefined)
  root = new MemoryNode('root'); memoryRenderer.render(h(Component), root); await flushComponent()
})
afterEach(() => memoryRenderer.render(null, root))

it('默认关闭且只选择图片；启用时仅保存视图配置', async () => {
  const toggle = findNode(root, n => n.props.role === 'switch')!
  expect(toggle.props['aria-checked']).toBe(false)
  toggle.props.onClick(); await flushComponent()
  expect(api.shelfConfigure).toHaveBeenCalledWith({ enabled: true, topHover: false, categories: ['image'] })
})

it('全部选择覆盖七类，可清空后仅选择链接和颜色', async () => {
  const all = findNode(root, n => n.type === 'input' && 'indeterminate' in n.props)!
  all.props.onChange({ target: { checked: true } }); await flushComponent()
  expect(api.shelfConfigure).toHaveBeenLastCalledWith({ enabled: false, topHover: false, categories: shelfCategories.map(c => c.key) })
  all.props.onChange({ target: { checked: false } }); await flushComponent()
  const link = findNode(root, n => n.type === 'input' && n.props.value === 'link')!
  link.props['onUpdate:modelValue'](['link', 'color']); link.props.onChange(); await flushComponent()
  expect(api.shelfConfigure).toHaveBeenLastCalledWith({ enabled: false, topHover: false, categories: ['link', 'color'] })
})

it('配置读取失败保持表单禁用，重新读取后恢复', async () => {
  api.shelfSnapshot.mockRejectedValueOnce(new Error('读取失败'))
  memoryRenderer.render(null, root); memoryRenderer.render(h(Component), root); await flushComponent()
  expect(findNode(root, n => n.type === 'fieldset')!.props.disabled).toBe(true)
  expect(findNode(root, n => n.props.role === 'alert')).toBeDefined()
  findNode(root, n => n.type === 'button' && n.children.some(c => c.text === '重新读取'))!.props.onClick()
  await flushComponent()
  expect(findNode(root, n => n.type === 'fieldset')!.props.disabled).toBe(false)
})
