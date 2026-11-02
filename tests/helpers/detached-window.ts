import { defineComponent, h } from 'vue'

/** 内存宿主只替代系统窗口边界，弹窗中的业务控件仍通过真实交互验证。 */
export const DetachedWindowStub = defineComponent({
  props: { open: Boolean, title: String },
  setup: (props, { slots, attrs, expose }) => {
    expose({ focus: () => {} })
    return () => props.open ? h('section', { ...attrs, 'data-window': props.title }, slots.default?.()) : null
  },
})
