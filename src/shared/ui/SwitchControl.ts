import { defineComponent, h } from 'vue'

/**
 * 设置页统一的开关：按钮语义的 role=switch，外观见 control-theme.css 的 .switch-control。
 * 受控组件，只发出期望值；需要先调用后端再生效的开关（开机自启、剪贴板记录）
 * 用 :model-value 加 @update:model-value 自行决定何时更新。id、aria-label 等属性透传到按钮，
 * 配合 <label for> 即可点击标签切换。
 */
export default defineComponent({
  name: 'SwitchControl',
  props: { modelValue: { type: Boolean, default: false }, disabled: Boolean, busy: Boolean },
  emits: ['update:modelValue', 'change'],
  setup(props, { emit }) {
    function toggle() {
      const next = !props.modelValue
      emit('update:modelValue', next)
      // 按钮不会触发原生 change；自动保存的表单通过这个事件提交。
      emit('change', next)
    }
    return () => h('button', {
      type: 'button', class: 'switch-control', role: 'switch',
      'aria-checked': props.modelValue, 'aria-busy': props.busy || undefined,
      disabled: props.disabled, onClick: toggle,
    }, h('span', { 'aria-hidden': 'true' }))
  },
})
