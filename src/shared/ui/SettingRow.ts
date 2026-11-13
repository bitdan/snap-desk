import { defineComponent, h } from 'vue'
import InfoTip from './InfoTip.vue'

/**
 * 设置行：左侧标签（可带说明提示与一行次级说明），右侧控件区；沿用全局 .setting-row 的行高与分隔线。
 * 默认插槽放控件和状态文字，for 对应控件 id，使点击标签等同于操作控件。
 */
export default defineComponent({
  name: 'SettingRow',
  props: { label: { type: String, required: true }, for: String, tip: String, hint: String },
  setup(props, { slots }) {
    return () => h('div', { class: 'setting-row' }, [
      h('div', { class: 'setting-row-label' }, [
        h('span', { class: 'setting-row-title' }, [
          h('label', { for: props.for }, props.label),
          props.tip ? h(InfoTip, { text: props.tip }) : null,
        ]),
        // 只放影响当前选择的一句话；完整说明归入帮助。
        props.hint ? h('small', { class: 'setting-row-hint' }, props.hint) : null,
      ]),
      h('div', { class: 'setting-row-control' }, slots.default?.()),
    ])
  },
})
