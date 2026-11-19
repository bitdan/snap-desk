import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
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
    validate: (capture: string) => validateShortcuts({ capture, pin: '', record: '', history: '', translate: '', shelf: '', ocr: '' }),
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

describe('macOS WebKit 录入', () => {
  let active: unknown = null
  const key = (key: string, code: string, modifiers: Partial<KeyboardEvent> = {}) => ({
    key, code, repeat: false, isComposing: false, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false,
    preventDefault: vi.fn(), stopPropagation: vi.fn(), ...modifiers,
  })
  beforeEach(() => {
    active = null
    // WebKit 点击按钮不会改变 activeElement，只有显式 focus() 才会。
    vi.stubGlobal('document', { hasFocus: () => true, get activeElement() { return active } })
  })
  async function mount(update = vi.fn()) {
    memoryRenderer.render(h(Component, { modelValue: '', label: '区域截图', validate: () => null, 'onUpdate:modelValue': update }), root)
    await flushComponent()
    const button = findNode(root, node => node.type === 'button')!
    // 模板引用是响应式代理；记录 focus 调用时的 this，与组件内比较的对象一致。
    button.focus.mockImplementation(function (this: unknown) { active = this })
    return { button, update }
  }

  it('点击未自动聚焦时主动聚焦并进入录入状态', async () => {
    const { button } = await mount()
    await button.props.onClick(); await flushComponent()
    expect(button.focus).toHaveBeenCalled()
    expect(editing).toHaveBeenCalledExactlyOnceWith(true)
    expect(button.props['aria-pressed']).toBe(true)
  })

  it('⌘ 组合缺少主键 keyup 时以 ⌘ 松开完成保存', async () => {
    const { button, update } = await mount()
    await button.props.onClick(); await flushComponent()
    button.props.onKeydown(key('Meta', 'MetaLeft', { metaKey: true }))
    button.props.onKeydown(key('Shift', 'ShiftLeft', { metaKey: true, shiftKey: true }))
    button.props.onKeydown(key('a', 'KeyA', { metaKey: true, shiftKey: true }))
    // WebKit 不派发 A 的 keyup，只收到修饰键依次松开。
    button.props.onKeyup(key('Shift', 'ShiftLeft', { metaKey: true }))
    await flushComponent()
    expect(update).not.toHaveBeenCalled()
    button.props.onKeyup(key('Meta', 'MetaLeft'))
    await flushComponent()
    expect(update).toHaveBeenCalledExactlyOnceWith('Shift+Super+A')
    expect(editing).toHaveBeenLastCalledWith(false)
  })
})
