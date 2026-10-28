import { afterEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick, ref } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { defaultInputEffects } from '../../src/features/recording/recording'

vi.mock('../../src/features/recording/components/RecordingOptionMenu.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('menu-stub') }) }
})
import Menu from '../../src/features/recording/components/RecordingSetupMenu.vue'
const SetupMenu = clientComponent(Menu, new URL('../../src/features/recording/components/RecordingSetupMenu.vue', import.meta.url))
const root = new MemoryNode('root')
afterEach(() => memoryRenderer.render(null, root))

describe('录屏主界面收起开关', () => {
  it('实际点击更新双向绑定，关闭后可重新开启', async () => {
    const hideMainWindow = ref(true)
    memoryRenderer.render(h({ setup: () => () => h(SetupMenu, { busy: false, format: 'mp4', encoding: 'auto', quality: 'balanced', mbps: '', countdown: 0,
      effects: defaultInputEffects(), hideMainWindow: hideMainWindow.value,
      'onUpdate:hideMainWindow': (value: boolean) => { hideMainWindow.value = value } }) }), root)
    const toggle = () => findNode(root, node => node.props.role === 'switch')!
    expect(toggle().props['aria-checked']).toBe(true)
    toggle().props.onClick(); await nextTick()
    expect(hideMainWindow.value).toBe(false)
    expect(toggle().props['aria-checked']).toBe(false)
    toggle().props.onClick(); await nextTick()
    expect(hideMainWindow.value).toBe(true)
  })
})
