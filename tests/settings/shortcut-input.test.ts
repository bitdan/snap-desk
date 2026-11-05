import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h, reactive } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { validateShortcuts } from '../../src/features/settings/settings'

const editing = vi.hoisted(() => vi.fn(async (_value: boolean) => {}))
vi.mock('../../src/services/api', () => ({ api: { setShortcutEditing: editing } }))
import ShortcutInput from '../../src/features/settings/components/ShortcutInput.vue'
const Component = clientComponent(ShortcutInput, new URL('../../src/features/settings/components/ShortcutInput.vue', import.meta.url))
let root: MemoryNode
beforeEach(() => {
  vi.clearAllMocks()
  root = new MemoryNode('root')
  vi.stubGlobal('window', new EventTarget())
  vi.stubGlobal('document', {
    hasFocus: () => true,
    get activeElement() { return reactive(findNode(root, node => node.type === 'button')!) },
  })
})
afterEach(async () => { memoryRenderer.render(null, root); await flushComponent(); vi.unstubAllGlobals() })

it.each([
  [' ', 'Space', { ctrlKey: true }, 'Ctrl+Space'],
  ['Delete', 'Delete', { altKey: true }, 'Alt+Delete'],
  ['+', 'Equal', { ctrlKey: true, shiftKey: true }, 'Ctrl+Shift+Equal'],
  ['a', 'KeyA', {}, 'A'],
])('真实录入 %s：拦截默认行为并等所有按键松开后提交', async (key, code, modifiers, expected) => {
  const update = vi.fn()
  memoryRenderer.render(h(Component, {
    modelValue: 'F1', label: '区域截图',
    validate: (capture: string) => validateShortcuts({ capture, pin: '', record: '', history: '', translate: '', shelf: '' }),
    'onUpdate:modelValue': update,
  }), root)
  await flushComponent()
  const button = findNode(root, node => node.type === 'button')!
  await button.props.onClick(); await flushComponent()
  expect(editing).toHaveBeenLastCalledWith(true)
  const event = { key, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, repeat: false, isComposing: false, preventDefault: vi.fn(), stopPropagation: vi.fn(), ...modifiers }
  button.props.onKeydown(event)
  await flushComponent()
  expect(event.preventDefault).toHaveBeenCalledOnce()
  expect(update).not.toHaveBeenCalled()
  button.props.onKeyup({ ...event, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false })
  await flushComponent()
  expect(editing).toHaveBeenLastCalledWith(false)
  expect(update).toHaveBeenCalledExactlyOnceWith(expected)
})
