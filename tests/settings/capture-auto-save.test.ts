import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), setShortcutEditing: vi.fn(), start: vi.fn() }))
const subscribe = vi.hoisted(() => vi.fn(async () => () => {}))
vi.mock('../../src/services/api', () => ({ api, desktop: true, onDelay: subscribe, onError: subscribe, onSettingsChanged: subscribe, onClipboardOpen: subscribe }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/appearance', () => ({ applyAppearance: vi.fn(), appearance: { value: { language: 'zh-CN' } } }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Compatibility from '../../src/features/settings/components/ShortcutCompatibility.vue'
clientComponent(Compatibility, new URL('../../src/features/settings/components/ShortcutCompatibility.vue', import.meta.url))
import App from '../../src/app/App.vue'
const Component = clientComponent(App, new URL('../../src/app/App.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
async function flush() { await vi.advanceTimersByTimeAsync(0) }
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('window', new EventTarget())
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  let settings = defaultSettings(); settings.onboardingVersion = 1
  api.status.mockImplementation(async () => ({ settings: structuredClone(settings), version: 'test', platform: 'windows', delayRemaining: 0 }))
  api.saveSettings.mockImplementation(async value => { settings = structuredClone(value); return structuredClone(settings) })
  memoryRenderer.render(h(Component), root)
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })

describe('截图设置自动生效', () => {
  it('格式选择后自动写入配置，页面没有保存按钮', async () => {
    await flush()
    findNode(root, node => node.props.id === 'export-format')!.props['onUpdate:modelValue']('webp')
    await vi.advanceTimersByTimeAsync(350)
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ export: expect.objectContaining({ format: 'webp' }) }))
    expect(findNode(root, node => node.type === 'button' && text(node) === '保存设置')).toBeUndefined()
  })
  it('点击开始截图会先提交尚在防抖中的文件名前缀', async () => {
    await flush()
    findNode(root, node => node.props.id === 'filename-prefix')!.props['onUpdate:modelValue']('Capture')
    findNode(root, node => node.type === 'form')!.props.onInput()
    await findNode(root, node => node.type === 'button' && text(node) === '开始截图')!.props.onClick()
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ export: expect.objectContaining({ filenamePrefix: 'Capture' }) }))
    expect(api.start).toHaveBeenCalledOnce()
  })
})
