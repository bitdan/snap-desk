import { readFileSync } from 'node:fs'
import { parse, compileScript } from '@vue/compiler-sfc'
import { compile } from '@vue/compiler-dom'
import * as Vue from 'vue'
import { vi } from 'vitest'

/** Node 环境的 Vue 插件使用 SSR 模板；统一补客户端 render，不抽取组件私有函数。 */
export function clientComponent(component: Vue.Component, file: URL) {
  const { descriptor } = parse(readFileSync(file, 'utf8'))
  const bindings = compileScript(descriptor, { id: file.pathname }).bindings
  Object.assign(component, { render: new Function('Vue', compile(descriptor.template!.content, { mode: 'function', prefixIdentifiers: true, bindingMetadata: bindings }).code)(Vue) })
  return Vue.defineComponent({ inheritAttrs: false, setup(_, { attrs, slots }) {
    Vue.provide(Vue.ssrContextKey, { modules: new Set() })
    return () => Vue.h(component, attrs, slots)
  } })
}

/** 共享内存媒体宿主：只模拟控件/媒体 API，不启动浏览器、读取录像或调用系统设备。 */
export class MemoryNode {
  parent: MemoryNode | null = null
  children: MemoryNode[] = []
  props: Record<string, any> = {}
  text = ''
  style: Record<string, string> = {}
  currentTime = 0
  ended = false
  muted = false
  playbackRate = 1
  value: unknown
  selectedIndex = -1
  getRootNode(): MemoryNode { return this.parent?.getRootNode() ?? this }
  get options() { return this.children.filter(child => child.type === 'option') }
  focus = vi.fn()
  load = vi.fn()
  pause = vi.fn(() => this.props.onPause?.())
  play = vi.fn(async () => { this.props.onPlay?.() })
  removeAttribute = vi.fn((key: string) => { delete this.props[key] })
  addEventListener = vi.fn()
  constructor(public type: string) {}
  closest(selector: string): MemoryNode | null { return selector.split(',').map(value => value.trim()).includes(this.type) ? this : this.parent?.closest(selector) ?? null }
}
export const memoryRenderer = Vue.createRenderer<MemoryNode, MemoryNode>({
  createElement: type => new MemoryNode(type), createText: text => Object.assign(new MemoryNode('#text'), { text }), createComment: type => new MemoryNode(type),
  setText: (node, text) => { node.text = text }, setElementText: (node, text) => { node.text = text; node.children = [] },
  parentNode: node => node.parent, nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
  insert(node, parent, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const index = anchor ? parent.children.indexOf(anchor) : -1
    if (index >= 0) parent.children.splice(index, 0, node); else parent.children.push(node)
  },
  remove(node) { node.parent?.children.splice(node.parent.children.indexOf(node), 1); node.parent = null },
  patchProp(node, key, _old, value) { node.props[key] = value; if (key === 'value') node.value = value },
})
export function findNode(root: MemoryNode, predicate: (node: MemoryNode) => boolean): MemoryNode | undefined {
  if (predicate(root)) return root
  for (const child of root.children) { const found = findNode(child, predicate); if (found) return found }
}
export async function flushComponent() { for (let i = 0; i < 8; i++) { await Promise.resolve(); await Vue.nextTick() } }
