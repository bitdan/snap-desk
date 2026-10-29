import { afterEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'
import { useAnnotations } from '../../src/features/annotations/use-annotations'

vi.mock('../../src/features/annotations/components/AnnotationToolbar.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('toolbar-stub') }) }
})
vi.mock('../../src/features/annotations/components/AnnotationSurface.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('surface-stub') }) }
})
vi.mock('../../src/features/annotations/components/AnnotationToolbarHost.vue', async importOriginal => {
  const actual = await importOriginal<typeof import('../../src/features/annotations/components/AnnotationToolbarHost.vue')>()
  const helper = await import('../helpers/component-host')
  return { default: helper.clientComponent(actual.default, new URL('../../src/features/annotations/components/AnnotationToolbarHost.vue', import.meta.url)) }
})
import Workspace from '../../src/features/annotations/components/AnnotationWorkspace.vue'
const AnnotationWorkspace = clientComponent(Workspace, new URL('../../src/features/annotations/components/AnnotationWorkspace.vue', import.meta.url))
const root = new MemoryNode('root')
afterEach(() => memoryRenderer.render(null, root))

describe('共享标注宿主的菜单激活', () => {
  it('菜单打开前恢复输入区域，普通工具及 Tab 不提前改变模式', () => {
    const activate = vi.fn(), bounds = { x: 0, y: 0, width: 640, height: 480 }
    const editor = useAnnotations(() => bounds, () => '', () => false)
    memoryRenderer.render(h(AnnotationWorkspace, { editor, bounds, width: 640, height: 480, surfaceVisible: false, toolbarVisible: true,
      enabled: false, active: false, disabled: false, onActivate: activate }), root)
    const toolbar = findNode(root, node => String(node.props.class).includes('capture-toolbar'))!
    const menuTarget = { closest: (selector: string) => selector === '[aria-haspopup="menu"]' ? {} : null }
    toolbar.props.onPointerdownCapture({ type: 'pointerdown', target: { closest: () => null } })
    toolbar.props.onKeydownCapture({ type: 'keydown', key: 'Tab', target: menuTarget })
    expect(activate).not.toHaveBeenCalled()
    toolbar.props.onPointerdownCapture({ type: 'pointerdown', target: menuTarget })
    expect(activate).toHaveBeenCalledOnce()
    toolbar.props.onKeydownCapture({ type: 'keydown', key: 'ArrowDown', target: menuTarget })
    expect(activate).toHaveBeenCalledTimes(2)
  })
})
