import { afterEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import Surface from '../../src/features/annotations/components/AnnotationSurface.vue'
import { useAnnotations } from '../../src/features/annotations/use-annotations'
import { clientComponent, findNode, MemoryNode, memoryRenderer } from '../helpers/component-host'

const Component = clientComponent(Surface, new URL('../../src/features/annotations/components/AnnotationSurface.vue', import.meta.url))
const root = new MemoryNode('root')
afterEach(() => memoryRenderer.render(null, root))
describe('预览黑边下的图形坐标', () => {
  it('通过 SVG 逆矩阵换算点击，黑边区域不生成标注', () => {
    const bounds = { x: 0, y: 0, width: 640, height: 480 }
    const editor = useAnnotations(() => bounds, () => '', () => false)
    editor.setTool('rectangle')
    memoryRenderer.render(h(Component, { editor, bounds, width: 640, height: 480, enabled: true, preserveAspectRatio: 'xMidYMid meet' }), root)
    const svg = findNode(root, node => node.type === 'svg')!
    Object.assign(svg, { getScreenCTM: () => ({ a: .5, b: 0, inverse: () => ({}) }), setPointerCapture: vi.fn(),
      createSVGPoint: () => ({ x: 0, y: 0, matrixTransform() { return { x: (this.x - 240) / .5, y: (this.y - 50) / .5 } } }) })
    const event = (clientX: number, clientY: number) => ({ clientX, clientY, button: 0, pointerId: 1, currentTarget: svg, target: { getAttribute: () => null }, preventDefault: vi.fn(), stopPropagation: vi.fn() })
    svg.props.onPointerdown(event(200, 70))
    expect(editor.draft).toBeNull()
    svg.props.onPointerdown(event(260, 70))
    expect(editor.draft?.start).toEqual({ x: 40, y: 40 })
  })
})
