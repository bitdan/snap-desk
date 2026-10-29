import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import SelectControl from '../../src/shared/ui/SelectControl.vue'

describe('统一下拉控件', () => {
  it('使用应用内触发器显示当前值，保留标签和禁用状态', async () => {
    const html = await renderToString(h(SelectControl, { modelValue: 0, disabled: true, 'aria-label': '延时', options: [{ value: 0, label: '无延时' }, { value: 3, label: '3 秒' }] }))
    expect(html).toContain('role="combobox"')
    expect(html).toContain('aria-label="延时"')
    expect(html).toContain('无延时')
    expect(html).toContain('disabled')
    expect(html).not.toContain('<select')
  })
  it('null 和空字符串保持不同的选择语义，业务标签原样显示', async () => {
    const options = [{ value: null, label: '全部贴图' }, { value: '', label: '未分组' }, { value: '中文分组', label: '中文分组' }]
    for (const option of options) {
      const html = await renderToString(h(SelectControl, { modelValue: option.value, options }))
      expect(html).toContain(option.label)
    }
  })
})
