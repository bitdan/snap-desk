import { expect, it, vi } from 'vitest'
import { h, ref } from 'vue'
import { findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import SwitchControl from '../../src/shared/ui/SwitchControl'
import SettingRow from '../../src/shared/ui/SettingRow'

it('开关是受控按钮：点击发出期望值和 change，属性透传到按钮', async () => {
  const root = new MemoryNode('root'), value = ref(false), change = vi.fn()
  memoryRenderer.render(h({ setup: () => () => h(SwitchControl, { id: 'demo', modelValue: value.value, 'onUpdate:modelValue': (next: boolean) => { value.value = next }, onChange: change }) }), root)
  const button = () => findNode(root, node => node.props.role === 'switch')!
  expect(button().type).toBe('button')
  expect(button().props).toMatchObject({ id: 'demo', 'aria-checked': false })
  button().props.onClick(); await flushComponent()
  expect(button().props['aria-checked']).toBe(true)
  expect(change).toHaveBeenCalledExactlyOnceWith(true)
  memoryRenderer.render(null, root)
})

it('设置行把标签关联到控件，控件放在右侧区域', () => {
  const root = new MemoryNode('root')
  memoryRenderer.render(h(SettingRow, { label: '开机自启', for: 'autostart' }, () => h(SwitchControl, { id: 'autostart' })), root)
  const label = findNode(root, node => node.type === 'label')!
  expect(label.props.for).toBe('autostart')
  expect(findNode(root, node => node.props.class === 'setting-row-control')!.children.some(node => node.props.id === 'autostart')).toBe(true)
  memoryRenderer.render(null, root)
})
