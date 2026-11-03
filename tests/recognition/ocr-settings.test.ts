import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { defaultSettings } from '../../src/features/settings/settings'
import type { SystemOcrStatus } from '../../src/features/recognition/ocr'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const api = vi.hoisted(() => ({ status: vi.fn(), saveSettings: vi.fn(), systemOcrStatus: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
vi.mock('../../src/features/resources/components/ResourceSettings.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('paddle-resources') }) }
})
import OcrSettings from '../../src/features/recognition/components/OcrSettings.vue'
const Component = clientComponent(OcrSettings, new URL('../../src/features/recognition/components/OcrSettings.vue', import.meta.url))
let root: MemoryNode, settings: ReturnType<typeof defaultSettings>
const available: SystemOcrStatus = { supported: true, error: null, languages: [{ tag: 'zh-Hans-CN', name: '简体中文' }, { tag: 'en-US', name: 'English' }] }
const text = (node: MemoryNode): string => node.text + node.children.map(text).join('')
const select = (label: string) => findNode(root, node => node.props['aria-label'] === label)!
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)!
async function mount() { memoryRenderer.render(h(Component), root); await flushComponent(); await flushComponent() }
beforeEach(() => {
  vi.resetAllMocks(); root = new MemoryNode('root'); settings = defaultSettings()
  api.status.mockImplementation(async () => ({ settings }))
  api.saveSettings.mockImplementation(async value => { settings = value; return value })
  api.systemOcrStatus.mockResolvedValue(available)
})
afterEach(() => { memoryRenderer.render(null, root) })

it('保留默认 PaddleOCR，切换系统引擎仅保存 OCR 配置并显示实际语言包', async () => {
  await mount()
  expect(select('OCR 引擎').value).toBe('local-ppocr')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeDefined()
  settings.ai.timeoutSeconds = 60; settings.shortcuts.capture = 'F8'
  select('OCR 引擎').props['onUpdate:modelValue']('windows-media'); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'auto' })
  expect(settings.ai.timeoutSeconds).toBe(60); expect(settings.shortcuts.capture).toBe('F8')
  expect(findNode(root, node => node.type === 'paddle-resources')).toBeUndefined()
  expect(select('识别语言').props.options.map((option: any) => option.value)).toEqual(['auto', 'zh-Hans-CN', 'en-US'])
  select('识别语言').props['onUpdate:modelValue']('en-US'); await flushComponent()
  expect(settings.ocr).toEqual({ provider: 'windows-media', language: 'en-US' })
})

it('系统错误与语言包缺失直接显示，保持用户选择且不写入 PaddleOCR', async () => {
  settings.ocr = { provider: 'windows-media', language: 'zh-Hans-CN' }
  api.systemOcrStatus.mockResolvedValueOnce({ supported: true, languages: [], error: 'Windows OCR 调用失败' })
  await mount()
  expect(text(root)).toContain('Windows OCR 调用失败')
  expect(select('OCR 引擎').value).toBe('windows-media')
  expect(select('识别语言').props.disabled).toBe(true)
  expect(api.saveSettings).not.toHaveBeenCalled()
  api.systemOcrStatus.mockResolvedValueOnce({ supported: true, languages: [], error: null })
  await button('刷新语言包').props.onClick(); await flushComponent()
  expect(text(root)).toContain('未安装 Windows OCR 语言包')
  expect(settings.ocr.provider).toBe('windows-media')
})

it('非 Windows 平台禁用系统引擎，但保留已有设置以便主动切换', async () => {
  settings.ocr.provider = 'windows-media'
  api.systemOcrStatus.mockResolvedValueOnce({ supported: false, languages: [], error: null })
  await mount()
  expect(select('OCR 引擎').props.options[1].disabled).toBe(true)
  expect(text(root)).toContain('Windows 系统 OCR 仅支持 Windows。')
  expect(api.saveSettings).not.toHaveBeenCalled()
  select('OCR 引擎').props['onUpdate:modelValue']('local-ppocr'); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
})

it('保存失败保留草稿并支持重试，不改写已生效的引擎', async () => {
  await mount()
  api.saveSettings.mockRejectedValueOnce(new Error('无法保存配置'))
  select('OCR 引擎').props['onUpdate:modelValue']('windows-media'); await flushComponent()
  expect(settings.ocr.provider).toBe('local-ppocr')
  expect(select('OCR 引擎').value).toBe('windows-media')
  expect(text(root)).toContain('无法保存配置')
  await button('重试').props.onClick(); await flushComponent()
  expect(settings.ocr.provider).toBe('windows-media')
})

it('语言包查询失败后可刷新恢复，不自动更换引擎或语言', async () => {
  settings.ocr = { provider: 'windows-media', language: 'en-US' }
  api.systemOcrStatus.mockRejectedValueOnce(new Error('语言包查询失败'))
  await mount()
  expect(text(root)).toContain('语言包查询失败')
  await button('刷新语言包').props.onClick(); await flushComponent()
  expect(text(root)).toContain('已安装 2 个 OCR 语言包')
  expect(settings.ocr.language).toBe('en-US')
  expect(api.saveSettings).not.toHaveBeenCalled()
})
