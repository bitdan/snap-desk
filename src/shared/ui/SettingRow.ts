import { defineComponent, h } from 'vue'
import InfoTip from './InfoTip.vue'

/**
 * 设置行：左侧标签（可带说明提示），右侧控件区；沿用全局 .setting-row 的行高与分隔线。
 * 默认插槽放控件和状态文字，for 对应控件 id，使点击标签等同于操作控件。
 */
export default defineComponent({
  name: 'SettingRow',
  props: { label: { type: String, required: true }, for: String, tip: String },
  setup(props, { slots }) {
    return () => h('div', { class: 'setting-row' }, [
      h('div', { class: 'setting-row-label' }, [
        h('label', { for: props.for }, props.label),
        props.tip ? h(InfoTip, { text: props.tip }) : null,
      ]),
      h('div', { class: 'setting-row-control' }, slots.default?.()),
    ])
  },
})
