import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import Surface from '../../src/features/annotations/components/AnnotationSurface.vue'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { handleWindowEscape } from '../../src/shared/ui/escape'
import { clientComponent, findNode, flushComponent, MemoryNode, memoryRenderer } from '../helpers/component-host'

const Component = clientComponent(Surface, new URL('../../src/features/annotations/components/AnnotationSurface.vue', import.meta.url))
const bounds = { x: 0, y: 0, width: 800, height: 600 }
let root: MemoryNode
const rasterize = vi.fn(async (shape: any) => ({ ...shape, end: { x: shape.start.x + 80, y: shape.start.y + 36 }, raster: 'glyph' }))
async function mount() {
  const editor = useAnnotations(() => bounds, () => '', () => false, undefined, rasterize)
  memoryRenderer.render(h(Component, { editor, bounds, width: 800, height: 600, enabled: true }), root)
  editor.setTool('text'); editor.begin({ x: 100, y: 100 }, 2); editor.draft!.text = '输入草稿'; await flushComponent()
  return { editor, key: (event: unknown) => findNode(root, node => node.type === 'foreignObject')!.props.onKeydown(event) }
}
const event = (key: string, patch = {}) => ({ key, stopPropagation: vi.fn(), preventDefault: vi.fn(), ...patch })
beforeEach(() => {
  root = new MemoryNode('root'); rasterize.mockClear()
  vi.stubGlobal('Document', class {}); vi.stubGlobal('ShadowRoot', class {})
  vi.spyOn(MemoryNode.prototype, 'getRootNode').mockReturnValue(root)
  vi.stubGlobal('document', { createElement: () => ({ getContext: () => ({ measureText: (text: string) => ({ width: text.length * 20 }) }) }) })
})
afterEach(() => { memoryRenderer.render(null, root); vi.restoreAllMocks(); vi.unstubAllGlobals() })

describe('文字输入键盘交互', () => {
  it('Esc 取消草稿并清理溢出错误，不再次保存或退出截图', async () => {
    const { editor, key } = await mount()
    rasterize.mockRejectedValueOnce(new Error('文字超出区域'))
    await editor.finishText(); expect(editor.error).toContain('文字超出区域')
    const escape = event('Escape'); key(escape); await flushComponent()
    expect(editor.textEditing).toBe(false); expect(editor.interacting).toBe(false)
    expect(editor.error).toBe(''); expect(editor.history.present).toEqual([])
    expect(rasterize).toHaveBeenCalledOnce()
    expect(escape.stopPropagation).toHaveBeenCalledOnce(); expect(escape.preventDefault).toHaveBeenCalledOnce()
  })
  it('Enter 保存，Shift+Enter 保留换行', async () => {
    const { editor, key } = await mount()
    key(event('Enter', { shiftKey: true })); await flushComponent()
    expect(editor.textEditing).toBe(true); expect(rasterize).not.toHaveBeenCalled()
    key(event('Enter')); await flushComponent()
    expect(editor.history.present[0].text).toBe('输入草稿'); expect(editor.textEditing).toBe(false)
  })
  it('焦点移出文本框后，窗口 Esc 仍先取消文字，二次编辑保留原文字', async () => {
    const { editor } = await mount()
    await editor.finishText(); editor.editText(0); editor.draft!.text = '未保存的修改'; await flushComponent()
    const exit = vi.fn()
    handleWindowEscape(event('Escape') as unknown as KeyboardEvent, exit); await flushComponent()
    expect(editor.textEditing).toBe(false); expect(editor.history.present[0].text).toBe('输入草稿')
    expect(exit).not.toHaveBeenCalled()
  })
  it.each([{ isComposing: true }, { keyCode: 229 }, { repeat: true }])('组词和长按不取消或保存：%o', async patch => {
    const { editor, key } = await mount(); key(event('Escape', patch)); await flushComponent()
    expect(editor.textEditing).toBe(true); expect(rasterize).not.toHaveBeenCalled()
  })
})
