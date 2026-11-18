import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'
import type { LocalTranslationStatus } from '../../src/features/recognition/ai'

const api = vi.hoisted(() => ({ translationModelStatus: vi.fn(), translationModelAction: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('../../src/shared/ui/button/Button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: (_, { attrs, slots }) => () => h('button', attrs, slots.default?.()) }) }
})
import Models from '../../src/features/recognition/components/LocalTranslationModels.vue'
const Component = clientComponent(Models, new URL('../../src/features/recognition/components/LocalTranslationModels.vue', import.meta.url))
const root = new MemoryNode('root')
const text = (node: MemoryNode = root): string => node.text + node.children.map(text).join('')
const button = (label: string) => findNode(root, node => node.type === 'button' && text(node).trim() === label)!
const ready: LocalTranslationStatus = { runtimeReady: true, runtimeSupported: true, models: [{ modelId: 'opus-mt-en-zh-int8', ready: true, downloadable: true, revision: '1', downloadBytes: 100 }] }
beforeEach(() => { vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {}) })
afterEach(() => { memoryRenderer.render(null, root); vi.resetAllMocks(); vi.unstubAllGlobals() })

it('查询失败时显示错误，重新打开后可卸载语言包并更新状态', async () => {
  api.translationModelStatus.mockRejectedValueOnce(new Error('状态查询失败'))
  memoryRenderer.render(h(Component), root); await flushComponent()
  expect(text()).toContain('状态查询失败')
  expect(text()).not.toContain('本地引擎已安装')
  memoryRenderer.render(null, root)
  api.translationModelStatus.mockResolvedValueOnce(ready)
  const changed = vi.fn()
  memoryRenderer.render(h(Component, { onChanged: changed }), root); await flushComponent()
  expect(text()).toContain('本地引擎已安装'); expect(text()).toContain('已校验')
  api.translationModelAction.mockResolvedValueOnce({ ...ready, models: [] })
  await button('卸载').props.onClick(); await flushComponent()
  expect(api.translationModelAction).toHaveBeenCalledWith('opus-mt-en-zh-int8', 'delete')
  expect(text()).not.toContain('已校验')
  expect(changed).toHaveBeenLastCalledWith({ ...ready, models: [] })
})

it('不支持的平台禁用引擎下载与导入', async () => {
  api.translationModelStatus.mockResolvedValueOnce({ ...ready, runtimeReady: false, runtimeSupported: false })
  memoryRenderer.render(h(Component), root); await flushComponent()
  expect(button('下载引擎').props.disabled).toBe(true)
  expect(text()).toContain('翻译引擎下载和导入目前仅支持 Windows x64。')
})
