import { afterEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { handleWindowEscape } from '../../src/shared/ui/escape'

vi.mock('reka-ui', async () => {
  const { defineComponent, h } = await import('vue')
  const root = defineComponent({ props: ['open'], emits: ['update:open'], setup: (props, { emit, slots }) => () => h('layer-root', { open: props.open, onToggle: (value: boolean) => emit('update:open', value) }, slots.default?.()) })
  const pass = defineComponent({ setup: (_, { attrs, slots }) => () => h('layer-part', attrs, slots.default?.()) })
  const names = ['SelectTrigger', 'SelectPortal', 'SelectContent', 'SelectViewport', 'SelectItem', 'SelectItemText', 'SelectItemIndicator', 'SelectScrollUpButton', 'SelectScrollDownButton',
    'PopoverTrigger', 'PopoverPortal', 'PopoverContent', 'DropdownMenuTrigger', 'DropdownMenuPortal', 'DropdownMenuContent', 'DropdownMenuItem']
  return { ...Object.fromEntries(names.map(name => [name, pass])), SelectRoot: root, PopoverRoot: root, DropdownMenuRoot: root }
})
import Select from '../../src/shared/ui/SelectControl.vue'
import Option from '../../src/features/recording/components/RecordingOptionMenu.vue'
import Source from '../../src/features/recording/components/RecordingSourceSelect.vue'
import Group from '../../src/features/annotations/components/AnnotationToolGroup.vue'
const component = (value: any, path: string) => clientComponent(value, new URL(path, import.meta.url))
const SelectControl = component(Select, '../../src/shared/ui/SelectControl.vue')
const cases = [
  { component: SelectControl, props: { modelValue: 0, options: [{ value: 0, label: '无延时' }] }, menu: 'select-menu' },
  { component: component(Option, '../../src/features/recording/components/RecordingOptionMenu.vue'), props: { label: '选项', disabled: false }, menu: 'recording-option-popup' },
  { component: component(Source, '../../src/features/recording/components/RecordingSourceSelect.vue'), props: { value: 'region', disabled: false }, menu: 'recording-source-menu' },
  { component: component(Group, '../../src/features/annotations/components/AnnotationToolGroup.vue'), props: { name: '形状', tools: ['rectangle', 'ellipse'], selected: 'rectangle', active: true, disabled: false }, menu: 'annotation-group-menu' },
]
const root = new MemoryNode('root')
afterEach(() => memoryRenderer.render(null, root))
function escape() {
  const event = { key: 'Escape', defaultPrevented: false, isComposing: false, repeat: false,
    preventDefault: vi.fn(() => { event.defaultPrevented = true }), stopPropagation: vi.fn() }
  return event
}

it.each(cases)('$menu 的 Esc 只关闭当前菜单，再按才退出窗口', async ({ component, props, menu }) => {
  memoryRenderer.render(h(component, props), root)
  const layer = () => findNode(root, node => node.type === 'layer-root')!
  layer().props.onToggle(true)
  await flushComponent()
  expect(layer().props.open).toBe(true)
  const event = escape(), exit = vi.fn()
  findNode(root, node => String(node.props.class).split(' ').includes(menu))!.props.onKeydown(event)
  await flushComponent()
  expect(layer().props.open).toBe(false)
  expect(event.defaultPrevented).toBe(true)
  handleWindowEscape(event as unknown as KeyboardEvent, exit)
  expect(exit).not.toHaveBeenCalled()
  handleWindowEscape(escape() as unknown as KeyboardEvent, exit)
  expect(exit).toHaveBeenCalledOnce()
})

it('已收起的下拉触发器不会吞掉所在窗口的 Esc', async () => {
  memoryRenderer.render(h(SelectControl, { modelValue: 0, options: [{ value: 0, label: '无延时' }] }), root)
  const event = escape(), exit = vi.fn()
  findNode(root, node => node.props.class === 'select-trigger')!.props.onKeydown(event)
  expect(event.stopPropagation).not.toHaveBeenCalled()
  handleWindowEscape(event as unknown as KeyboardEvent, exit)
  expect(exit).toHaveBeenCalledOnce()
})
