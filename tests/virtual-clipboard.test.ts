import { afterEach, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import * as Vue from 'vue'
import { createRenderer, h, nextTick, ref } from 'vue'
import * as virtualList from '../src/lib/virtual-list'
import type { ClipboardItem } from '../src/lib/clipboard'

// Node 环境的默认 SFC 转换面向 SSR；这里显式生成客户端 render，交给自定义宿主执行。
const descriptor = parse(readFileSync('src/components/clipboard/VirtualClipboardList.vue', 'utf8')).descriptor
const source = compileScript(descriptor, { id: 'virtual-list-test', inlineTemplate: true }).content
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const module = { exports: {} as { default: Vue.DefineComponent } }
new Function('require', 'exports', compiled)((name: string) => {
  if (name === 'vue') return Vue
  if (name === '@/lib/virtual-list') return virtualList
  throw new Error(`Unexpected component import: ${name}`)
}, module.exports)
const VirtualClipboardList = module.exports.default

// 使用 Vue 自定义宿主验证挂载/卸载与跨窗口焦点，不启动浏览器或依赖固定行高。
class ElementNode {
  children: ElementNode[] = []
  parent: ElementNode | null = null
  dataset: Record<string, string> = {}
  className = ''
  scrollTop = 0
  clientHeight = 300
  focus = vi.fn()
  scrollIntoView = vi.fn()
  constructor(public tag: string) {}
  querySelector(selector: string): ElementNode | undefined {
    for (const child of this.children) {
      if (selector === '.history-item' && child.className === 'history-item') return child
      const nested = child.querySelector(selector)
      if (nested) return nested
    }
  }
}
afterEach(() => vi.unstubAllGlobals())

it('mounts a bounded range and moves keyboard focus to an unmounted final row', async () => {
  vi.stubGlobal('HTMLElement', ElementNode)
  const disconnect = vi.fn()
  vi.stubGlobal('ResizeObserver', class { observe = vi.fn(); unobserve = vi.fn(); disconnect = disconnect })
  const renderer = createRenderer<ElementNode, ElementNode>({
    createElement: tag => new ElementNode(tag),
    createText: () => new ElementNode('text'), createComment: () => new ElementNode('comment'),
    setText: () => {}, setElementText: () => {},
    parentNode: node => node.parent,
    nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
    insert(node, parent, anchor) {
      if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
      node.parent = parent
      const index = anchor ? parent.children.indexOf(anchor) : -1
      if (index >= 0) parent.children.splice(index, 0, node)
      else parent.children.push(node)
    },
    remove(node) { node.parent?.children.splice(node.parent.children.indexOf(node), 1); node.parent = null },
    patchProp(node, key, _previous, value) {
      if (key === 'class') node.className = value
      if (key.startsWith('data-')) node.dataset[key.slice(5).replace(/-([a-z])/g, (_, char: string) => char.toUpperCase())] = String(value)
    },
  })
  const items = ref(Array.from({ length: 5000 }, (_, id) => ({ id, kind: id % 2 ? 'image' : 'text' } as ClipboardItem)))
  const list = ref<{ focusIndex(index: number): Promise<void>; resetScroll(): void } | null>(null)
  const app = renderer.createApp({ setup: () => () => h(VirtualClipboardList, { ref: list, items: items.value }, {
    default: ({ item }: { item: ClipboardItem }) => h('article', { class: 'history-item', 'data-id': item.id }),
  }) })
  const host = new ElementNode('host')
  app.mount(host)
  try {
    const viewport = host.children[0]
    const rows = () => viewport.children.filter(node => node.dataset.historyId !== undefined)
    expect(rows().length).toBeLessThan(20)
    expect(rows().some(row => row.dataset.historyId === '4999')).toBe(false)
    await list.value!.focusIndex(4999)
    expect(rows().length).toBeLessThan(20)
    expect(rows().find(row => row.dataset.historyId === '4999')!.querySelector('.history-item')!.focus).toHaveBeenCalled()
    await list.value!.focusIndex(4998)
    expect(rows().find(row => row.dataset.historyId === '4998')!.querySelector('.history-item')!.focus).toHaveBeenCalled()
    list.value!.resetScroll()
    items.value = items.value.slice(0, 40)
    await nextTick()
    expect(viewport.scrollTop).toBe(0)
    expect(rows()[0].dataset.historyId).toBe('0')
  } finally { app.unmount() }
  expect(disconnect).toHaveBeenCalledOnce()
})
