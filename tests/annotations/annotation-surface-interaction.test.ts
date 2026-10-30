import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import Surface from '../../src/features/annotations/components/AnnotationSurface.vue'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import type { Annotation } from '../../src/features/annotations/annotations'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const Component = clientComponent(Surface, new URL('../../src/features/annotations/components/AnnotationSurface.vue', import.meta.url))
const bounds = { x: 20, y: 30, width: 600, height: 700 }
let root: MemoryNode
const rasterizer = vi.fn(async (shape: Annotation) => ({ ...shape, end: { x: shape.start.x + 100, y: shape.start.y + 40 }, raster: 'glyph' }))
function mount(shapes: Annotation[] = [], onBackgroundPointerdown = vi.fn()) {
  const editor = useAnnotations(() => bounds, () => '', () => false, undefined, rasterizer)
  editor.reset(shapes)
  memoryRenderer.render(h(Component, { editor, bounds, width: 800, height: 800, enabled: true, onBackgroundPointerdown }), root)
  const svg = findNode(root, node => node.type === 'svg')!
  Object.assign(svg, { getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 800 }), setPointerCapture: vi.fn() })
  const event = (x: number, y: number) => ({ clientX: x, clientY: y, button: 0, buttons: 0, pointerId: 1, currentTarget: svg,
    target: { getAttribute: () => null }, preventDefault: vi.fn(), stopPropagation: vi.fn() })
  return { editor, svg, event }
}
beforeEach(() => {
  root = new MemoryNode('root'); rasterizer.mockClear()
  vi.stubGlobal('Document', class {})
  vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  vi.stubGlobal('document', { activeElement: null, createElement: () => ({ getContext: () => ({ measureText: (text: string) => ({ width: [...text].length * 20 }) }) }) })
  Object.defineProperty(MemoryNode.prototype, 'scrollHeight', { configurable: true, get: () => 68 })
})
afterEach(() => { memoryRenderer.render(null, root); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (MemoryNode.prototype as any).scrollHeight })

describe('标注画布真实指针交互', () => {
  it('指针工具只把空白拖动交给选区，图形命中和绘图工具保持原操作', async () => {
    const background = vi.fn()
    const { editor, svg, event } = mount([{ kind: 'ellipse', start: { x: 80, y: 90 }, end: { x: 180, y: 150 }, width: 3, color: '#ef4444' }], background)
    svg.props.onPointerdown(event(300, 300))
    expect(background).toHaveBeenCalledOnce()
    expect(editor.interacting).toBe(false)
    svg.props.onPointerdown(event(130, 120)); await svg.props.onPointerup(event(130, 120))
    expect(background).toHaveBeenCalledOnce()
    expect(editor.selected).toBe(0)
    editor.setTool('rectangle'); svg.props.onPointerdown(event(300, 300))
    expect(editor.interacting).toBe(true)
    expect(background).toHaveBeenCalledOnce()
  })
  it.each(['ellipse', 'rectangle', 'arrow', 'text'] as const)('%s 在绘图工具下仍能单击选中、拖动并撤销', async kind => {
    const shape: Annotation = { kind, start: { x: 80, y: 90 }, end: { x: 180, y: 150 }, width: 3, color: '#ef4444', ...(kind === 'text' ? { text: '已有文字', raster: 'glyph' } : {}) }
    const { editor, svg, event } = mount([shape])
    editor.setTool('rectangle')
    const origin = ['ellipse', 'rectangle'].includes(kind) ? { x: 130, y: 120 } : { x: 80, y: 90 }
    svg.props.onPointerdown(event(origin.x, origin.y)); await svg.props.onPointerup(event(0, 0))
    expect(editor.selected).toBe(0); expect(editor.textEditing).toBe(false)
    expect(editor.history.past).toEqual([])
    await flushComponent()
    expect(findNode(root, node => node.props['data-handle'] === 'se' || node.props['data-handle'] === 'end')).toBeDefined()
    svg.props.onPointerdown(event(origin.x, origin.y)); svg.props.onPointermove(event(origin.x + 30, origin.y + 20)); await svg.props.onPointerup(event(0, 0))
    expect(editor.history.present).toHaveLength(1)
    expect(editor.history.present[0].start).toEqual({ x: 110, y: 110 })
    await editor.travel('undo'); expect(editor.history.present).toEqual([shape])
  })

  it('文字单击选中，双击编辑，移出完成；长文字横向自适应且不再限制 180 高度', async () => {
    const shape: Annotation = { kind: 'text', start: { x: 80, y: 90 }, end: { x: 180, y: 130 }, width: 3, color: '#ef4444', text: '原文', raster: 'glyph' }
    const { editor, svg, event } = mount([shape])
    svg.props.onPointerdown(event(90, 100)); await svg.props.onPointerup(event(0, 0))
    expect(editor.textEditing).toBe(false)
    svg.props.onDblclick(event(90, 100)); await flushComponent()
    expect(editor.textEditing).toBe(true)
    const input = findNode(root, node => node.type === 'textarea')!
    input.props['onUpdate:modelValue']('这是一行明显超过原始文本框宽度的文字'); await flushComponent()
    expect(editor.textBox().width).toBeGreaterThan(180)
    expect(editor.textBox().height).toBe(640)
    const foreign = findNode(root, node => node.type === 'foreignObject')!
    foreign.props.onPointerleave({ buttons: 0 }); await flushComponent()
    expect(editor.textEditing).toBe(false)
    expect(editor.history.present).toHaveLength(1)
    expect(editor.history.present[0].text).toBe('这是一行明显超过原始文本框宽度的文字')
    await editor.travel('undo'); expect(editor.history.present[0].text).toBe('原文')
  })

  it('文字框起点固定，只向右下扩展，空间用尽停止新增并允许删改', async () => {
    const { editor, svg, event } = mount()
    editor.setTool('text'); svg.props.onPointerdown(event(400, 650)); await svg.props.onPointerup(event(400, 650)); await flushComponent()
    const input = findNode(root, node => node.type === 'textarea')!
    const type = async (value: string) => {
      input.value = value; input.props['onUpdate:modelValue'](value)
      input.props.onInput({ target: input, isComposing: false }); await flushComponent()
    }
    const origin = editor.textBox()
    await type('文'.repeat(15))
    const box = editor.textBox()
    expect(box.x).toBe(origin.x); expect(box.y).toBe(origin.y)
    expect(box.width).toBeGreaterThanOrEqual(origin.width)
    expect(box.x + box.width).toBeLessThanOrEqual(620)
    expect(editor.textMeasuredHeight).toBeGreaterThan(36)
    await type('文'.repeat(45))
    expect(editor.draft!.text).toBe('文'.repeat(15)); expect(input.value).toBe('文'.repeat(15))
    expect(input.props.title).toBe('已到选区边界，可缩小字号继续输入')
    await type('改'); expect(editor.draft!.text).toBe('改')
    expect(editor.textBox().x).toBe(origin.x); expect(editor.textBox().y).toBe(origin.y)
    findNode(root, node => node.type === 'foreignObject')!.props.onPointerleave({ buttons: 0 }); await flushComponent()
    expect(editor.textEditing).toBe(false); expect(editor.error).toBe('')
  })
  it('输入法组词和选字拖拽不提交，组词结束后移出才保存最终输入', async () => {
    const { editor, svg, event } = mount()
    editor.setTool('text'); svg.props.onPointerdown(event(80, 90)); await svg.props.onPointerup(event(0, 0)); await flushComponent()
    const input = findNode(root, node => node.type === 'textarea')!
    const foreign = findNode(root, node => node.type === 'foreignObject')!
    input.props['onUpdate:modelValue']('组词'); input.props.onCompositionstart()
    foreign.props.onPointerleave({ buttons: 0 }); await flushComponent()
    expect(rasterizer).not.toHaveBeenCalled()
    input.props['onUpdate:modelValue']('组词完成'); input.props.onCompositionend(); await flushComponent()
    expect(editor.history.present[0].text).toBe('组词完成')
    editor.setTool('text'); svg.props.onPointerdown(event(300, 300)); await svg.props.onPointerup(event(0, 0)); await flushComponent()
    findNode(root, node => node.type === 'textarea')!.props['onUpdate:modelValue']('选字')
    findNode(root, node => node.type === 'foreignObject')!.props.onPointerleave({ buttons: 1 }); await flushComponent()
    expect(editor.textEditing).toBe(true)
    expect(editor.history.present).toHaveLength(1)
  })
})
