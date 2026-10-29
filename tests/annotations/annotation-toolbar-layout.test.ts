import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { readFileSync } from 'node:fs'
import { parse, compileScript } from '@vue/compiler-sfc'
import { compile } from '@vue/compiler-dom'
import ts from 'typescript'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { annotationTools } from '../../src/features/annotations/annotation-tools'

// 本测试只验证工具栏布局；自定义图标用占位组件，避免 SSR 编译产物缺少客户端 render。
vi.mock('../../src/shared/ui/SelectControl.vue', async () => ({ default: (await import('../helpers/select-control')).SelectControlStub }))
vi.mock('../../src/features/annotations/components/ArrowTextIcon.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('svg') }) }
})
vi.mock('../../src/features/annotations/components/AnnotationToolGroup.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ props: ['name'], setup: props => () => h('tool-group', { name: props.name }) }) }
})
import Toolbar from '../../src/features/annotations/components/AnnotationToolbar.vue'
const { descriptor } = parse(readFileSync('src/features/annotations/components/AnnotationToolbar.vue', 'utf8'))
const bindings = compileScript(descriptor, { id: 'toolbar-layout-test' }).bindings
const renderCode = compile(descriptor.template!.content, { mode: 'function', prefixIdentifiers: true, bindingMetadata: bindings, expressionPlugins: ['typescript'] }).code
Object.assign(Toolbar, { render: new Function('Vue', ts.transpileModule(renderCode, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText)(Vue) })
interface Node { type: string; props: Record<string, any>; children: Node[]; parent: Node | null; text: string }
const node = (type: string): Node => ({ type, props: {}, children: [], parent: null, text: '' })
const renderer = Vue.createRenderer<Node, Node>({
  createElement: node, createText: text => ({ ...node('#text'), text }), createComment: node,
  setText: (n, text) => { n.text = text }, setElementText: (n, text) => { n.text = text },
  parentNode: n => n.parent, nextSibling: n => n.parent?.children[n.parent.children.indexOf(n) + 1] ?? null,
  patchProp: (n, key, _old, value) => { n.props[key] = value },
  insert(child, parent, anchor) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = parent; const i = anchor ? parent.children.indexOf(anchor) : -1; if (i < 0) parent.children.push(child); else parent.children.splice(i, 0, child) },
  remove(child) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null },
})
const root = node('root')
function all(test: (n: Node) => boolean, current = root): Node[] {
  return [...(test(current) ? [current] : []), ...current.children.flatMap(n => all(test, n))]
}
const row = (name: string) => all(n => n.props['aria-label'] === name)[0]
const storage = { getItem: vi.fn(), setItem: vi.fn() }
let editor: ReturnType<typeof useAnnotations>
beforeEach(() => {
  vi.resetAllMocks()
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('localStorage', storage)
  editor = useAnnotations(() => ({ x: 0, y: 0, width: 200, height: 100 }), () => '', () => false)
})
afterEach(() => { renderer.render(null, root); vi.unstubAllGlobals() })
async function mount(collapsible = true, defaultExpanded = false, pointerLabel?: string, active = true) {
  const Wrapper = { setup() {
    Vue.provide(Vue.ssrContextKey, { modules: new Set() })
    return () => Vue.h(Toolbar, { editor, active, disabled: false, collapsible, defaultExpanded, pointerLabel, onTool: editor.setTool }, { actions: () => Vue.h('button', { 'data-action': 'save' }, 'Save') })
  } }
  renderer.render(Vue.h(Wrapper), root); await Vue.nextTick()
}

describe('共享标注工具栏分行与折叠', () => {
  it('录屏桌面指针在展开和折叠时都显示明确名称与选中状态', async () => {
    editor.setTool('pen')
    await mount(true, true, '操作桌面', false)
    const pointer = row('操作桌面')
    expect(pointer.props['aria-pressed']).toBe(true)
    expect(pointer.props['data-annotation-tool']).toBe('edit')
    row('折叠工具栏').props.onClick(); await Vue.nextTick()
    const group = all(n => n.type === 'tool-group' && n.props.label === '操作桌面')[0]
    expect(group.props.active).toBe(true)
    expect(group.props.selected).toBe('edit')
  })
  it('清空和撤销在第一行，保存等操作放在第二行', async () => {
    await mount()
    expect(all(n => n.props.role === 'toolbar')).toHaveLength(2)
    const clear = all(n => n.props['aria-label'] === '清空标注', row('标注工具'))
    expect(clear).toHaveLength(1)
    expect(clear[0].props.disabled).toBe(true)
    const actions = row('标注操作').children.filter(n => n.type === 'button')
    expect(actions[0].props['data-action']).toBe('save')
    expect(all(n => n.props['aria-label'] === '清空标注', row('标注操作'))).toHaveLength(0)
    expect(all(n => n.props['aria-label'] === '撤销', row('标注工具'))).toHaveLength(1)
  })
  it('展开替换同一行的分组，收起不丢失工具选择或第二行操作', async () => {
    await mount()
    expect(all(n => n.type === 'tool-group')).toHaveLength(7)
    row('展开工具栏').props.onClick(); await Vue.nextTick()
    expect(all(n => n.type === 'tool-group')).toHaveLength(0)
    expect(all(n => n.props['data-annotation-tool'])).toHaveLength(annotationTools.length)
    expect(all(n => n.props.role === 'toolbar')).toHaveLength(2)
    const pen = all(n => n.props['data-annotation-tool'] === 'pen')[0]
    pen.props.onClick({ detail: 0 }); await Vue.nextTick()
    expect(editor.tool).toBe('pen')
    row('折叠工具栏').props.onClick(); await Vue.nextTick()
    expect(editor.tool).toBe('pen')
    expect(all(n => n.type === 'tool-group')).toHaveLength(7)
    expect(all(n => n.props['data-action'] === 'save', row('标注操作'))).toHaveLength(1)
    expect(storage.setItem).toHaveBeenLastCalledWith('snap-desk.capture-toolbar-expanded.v1', 'false')
  })
  it('非折叠宿主也保持相同的两行分工', async () => {
    await mount(false)
    expect(all(n => n.props['data-annotation-tool'], row('标注工具'))).toHaveLength(annotationTools.length)
    expect(all(n => n.props['aria-label'] === '清空标注', row('标注工具'))).toHaveLength(1)
  })
  it('宿主默认展开尊重此前保存的折叠偏好', async () => {
    storage.getItem.mockReturnValue('false')
    await mount(true, true)
    expect(all(n => n.type === 'tool-group')).toHaveLength(7)
    expect(row('展开工具栏').props['aria-expanded']).toBe(false)
  })
  it('首次使用遵循宿主默认值，跨窗口变更同步展开状态', async () => {
    await mount(true, true)
    expect(all(n => n.props['data-annotation-tool'])).toHaveLength(annotationTools.length)
    storage.getItem.mockReturnValue('false')
    const sync = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'storage')![1] as (event: StorageEvent) => void
    sync({ key: 'snap-desk.capture-toolbar-expanded.v1' } as StorageEvent)
    await Vue.nextTick()
    expect(all(n => n.type === 'tool-group')).toHaveLength(7)
  })
})
