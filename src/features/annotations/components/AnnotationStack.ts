import { defineComponent, h, type PropType, type VNode } from 'vue'
import type { Annotation } from '@/features/annotations/annotations'

/** 将已合成的标注组整体擦除，保持半透明交叠处与 Rust 导出一致。 */
export default defineComponent({
  props: {
    shapes: { type: Array as PropType<Annotation[]>, required: true },
    maskPrefix: { type: String, required: true },
  },
  setup(props, { slots }) {
    return () => {
      let layers: VNode[] = []
      props.shapes.forEach((s, index) => {
        if ((s.opacity ?? 1) <= 0) return
        if (s.kind === 'erase') layers = [h('g', { mask: `url(#${props.maskPrefix}-erase-${index})` }, layers)]
        else layers.push(...(slots.default?.({ s, index }) ?? []))
      })
      return h('g', layers)
    }
  },
})
