vi.mock('../../src/shared/ui/DetachedWindow.vue', async () => ({ default: (await import('../helpers/detached-window')).DetachedWindowStub }))
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultSettings } from '../../src/features/settings/settings'
import type { AppUpdateView } from '../../src/features/settings/types'

const api = vi.hoisted(() => ({ readTranslationInput: vi.fn(async () => null), status: vi.fn(), saveSettings: vi.fn(), setShortcutEditing: vi.fn(), start: vi.fn(), appUpdateStatus: vi.fn(), completeOnboarding: vi.fn() }))
const updateEvent = vi.hoisted(() => ({ receive: (_view: AppUpdateView) => {} }))
vi.mock('../../src/features/settings/events', () => ({ onUpdateWindowRequested: () => () => {}, onAppUpdate: vi.fn(async (fn: typeof updateEvent.receive) => {
  updateEvent.receive = fn
  return () => { updateEvent.receive = () => {} }
}) }))
const recordingPage = vi.hoisted(() => ({ loaded: vi.fn(), mounted: vi.fn() }))
const subscribe = vi.hoisted(() => vi.fn(async () => () => {}))
const importedEvent = vi.hoisted(() => ({ receive: (_settings: any) => {} }))
const subscribeImported = vi.hoisted(() => vi.fn(async (fn: (settings: any) => void) => {
  importedEvent.receive = fn
  return () => { importedEvent.receive = () => {} }
}))
vi.mock('../../src/services/api', () => ({ api, desktop: true, onDelay: subscribe, onError: subscribe, onSettingsChanged: subscribe, onSettingsImported: subscribeImported, onTranslationOpen: subscribe, onClipboardOpen: subscribe }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/appearance', () => ({ applyAppearance: vi.fn(), appearance: { value: { language: 'zh-CN' } } }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/recording/components/RecordingPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  recordingPage.loaded()
  return { __esModule: true, default: defineComponent({ setup() {
    recordingPage.mounted()
    return () => h('recording-page-stub')
  } }) }
})
vi.mock('../../src/features/settings/components/ConfigTransferSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  const { defaultSettings } = await import('../../src/features/settings/settings')
  return { __esModule: true, default: defineComponent({ props: ['prepareTransfer'], setup: (props) => () => h('button', {
    'data-import': true,
    onClick: async () => {
      await props.prepareTransfer()
      const settings = defaultSettings(); settings.export.filenamePrefix = 'Imported'; settings.shortcutsEnabled = false; settings.onboardingVersion = 1
      importedEvent.receive(settings)
    },
  }, 'Import') }) }
})
import Compatibility from '../../src/features/settings/components/ShortcutCompatibility.vue'
vi.mock('../../src/app/components/OnboardingGuide.vue', async () => {
  const { defineComponent, h, onMounted } = await import('vue')
  return { default: defineComponent({ props: ['busy', 'error'], emits: ['complete', 'navigate'], setup: (props, { emit }) => {
    onMounted(() => emit('navigate', '截图'))
    return () => h('section', { 'data-guide': true }, [
      h('button', { 'data-finish-guide': true, disabled: props.busy, onClick: () => emit('complete') }, 'Finish'),
      props.error ? h('p', { role: 'alert' }, props.error) : null,
    ])
  } }) }
})
clientComponent(Compatibility, new URL('../../src/features/settings/components/ShortcutCompatibility.vue', import.meta.url))
import SupportPages from '../../src/app/components/SupportPages.vue'
clientComponent(SupportPages, new URL('../../src/app/components/SupportPages.vue', import.meta.url))
import UpdateNotice from '../../src/features/settings/components/UpdateNotice.vue'
clientComponent(UpdateNotice, new URL('../../src/features/settings/components/UpdateNotice.vue', import.meta.url))
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
  api.appUpdateStatus.mockResolvedValue({ availableVersion: null })
  api.saveSettings.mockImplementation(async value => { settings = structuredClone(value); return structuredClone(settings) })
  api.completeOnboarding.mockImplementation(async () => { settings.onboardingVersion = 1; return structuredClone(settings) })
  memoryRenderer.render(h(Component), root)
})
afterEach(() => { memoryRenderer.render(null, root); vi.clearAllTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })

describe('截图设置自动生效', () => {
  it('帮助页可重开引导，完成保存失败保留引导，重试成功后关闭', async () => {
    await flush()
    const help = () => findNode(root, node => node.type === 'button' && text(node) === '帮助')!
    help().props.onClick(); await flush()
    const restart = () => findNode(root, node => node.type === 'button' && text(node) === '重新打开新手引导')!
    restart().props.onClick(); await flush()
    expect(findNode(root, node => node.props['data-guide'])).toBeDefined()
    api.completeOnboarding.mockRejectedValueOnce(new Error('无法保存引导状态'))
    findNode(root, node => node.props['data-finish-guide'])!.props.onClick(); await flush()
    expect(text(root)).toContain('无法保存引导状态')
    expect(findNode(root, node => node.props['data-guide'])).toBeDefined()
    findNode(root, node => node.props['data-finish-guide'])!.props.onClick(); await flush()
    expect(api.completeOnboarding).toHaveBeenCalledTimes(2)
    expect(findNode(root, node => node.props['data-guide'])).toBeUndefined()
    help().props.onClick(); await flush()
    restart().props.onClick(); await flush()
    expect(findNode(root, node => node.props['data-guide'])).toBeDefined()
    expect(text(root)).not.toContain('无法保存引导状态')
  })
  it('有更新时显示简洁下载提示和关于导航红点，更新消失后清除', async () => {
    await flush()
    const about = findNode(root, node => node.type === 'button' && text(node) === '关于')!
    const dot = () => findNode(about, node => String(node.props.class).includes('nav-update-dot'))
    expect(dot()).toBeUndefined()
    const view: AppUpdateView = { currentVersion: '0.1.9', availableVersion: '0.2.0', notes: '不展示的日志', packageKind: 'nsis',
      supported: true, configured: true, phase: 'available', downloaded: 0, total: null, error: null }
    updateEvent.receive(view); await flush()
    expect(dot()?.props['aria-label']).toBe('发现新版本')
    expect(findNode(root, node => node.type === 'button' && text(node).includes('发现新版本'))).toBeUndefined()
    const update = findNode(root, node => node.type === 'button' && text(node) === '有更新')!
    expect(update).toBeDefined()
    update.props.onClick(); await flush()
    expect(findNode(root, node => node.type === 'button' && text(node) === '下载更新')).toBeDefined()
    expect(text(root)).not.toContain(view.notes)
    updateEvent.receive({ ...view, availableVersion: null }); await flush()
    expect(dot()).toBeUndefined()
  })
  it('悬停和聚焦录屏导航只预取模块，点击后才挂载页面', async () => {
    await flush()
    const recording = findNode(root, node => node.type === 'button' && text(node) === '录屏')!
    recording.props.onPointerenter(); recording.props.onFocus()
    await vi.dynamicImportSettled(); await flush()
    expect(recordingPage.loaded).toHaveBeenCalledOnce()
    expect(recordingPage.mounted).not.toHaveBeenCalled()
    expect(findNode(root, node => node.props.id === 'filename-prefix')).toBeDefined()
    recording.props.onClick(); await vi.dynamicImportSettled(); await flush()
    expect(recordingPage.mounted).toHaveBeenCalledOnce()
    expect(findNode(root, node => node.type === 'recording-page-stub')).toBeDefined()
  })
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

it('导入成功替换截图页草稿和快捷键状态，返回原页面不显示旧配置', async () => {
  await flush()
  findNode(root, node => node.type === 'button' && text(node) === '导入/导出')!.props.onClick()
  await vi.dynamicImportSettled(); await flush()
  const importedSettings = defaultSettings(); importedSettings.export.filenamePrefix = 'Imported'; importedSettings.shortcutsEnabled = false; importedSettings.onboardingVersion = 1
  api.status.mockResolvedValue({ settings: importedSettings, version: 'test', platform: 'windows', delayRemaining: 0 })
  await findNode(root, node => node.props['data-import'])!.props.onClick()
  await flush()
  findNode(root, node => node.type === 'button' && text(node) === '截图')!.props.onClick()
  await flush()
  expect(findNode(root, node => node.props.id === 'filename-prefix')!.value).toBe('Imported')
  expect(findNode(root, node => node.type === 'button' && text(node).includes('快捷键') && text(node).includes('已禁用'))).toBeDefined()
})
