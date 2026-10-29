import { defineComponent, h } from 'vue'

/** 业务交互测试替身：保留选项值与 v-model，弹出层和键盘行为由 Reka UI 承担。 */
export const SelectControlStub = defineComponent({
  inheritAttrs: false,
  props: ['modelValue', 'options', 'disabled'],
  emits: ['update:modelValue'],
  setup: (props, { attrs, emit }) => () => h('select-control', {
    ...attrs, value: props.modelValue, options: props.options, disabled: props.disabled,
    'onUpdate:modelValue': (value: string | number | null) => emit('update:modelValue', value),
  }),
})
