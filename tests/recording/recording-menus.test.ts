import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import RecordingSetupMenu from '../../src/features/recording/components/RecordingSetupMenu.vue'
import RecordingSourceSelect from '../../src/features/recording/components/RecordingSourceSelect.vue'
import { defaultInputEffects } from '../../src/features/recording/recording'

describe('紧凑录屏下拉入口', () => {
  it('默认只显示当前值和特效入口，不把复选框及自定义输入铺到工具行', async () => {
    const html = await renderToString(h(RecordingSetupMenu, { busy: false, format: 'mp4', quality: 'balanced', encoding: 'auto', mbps: '', countdown: 0, effects: defaultInputEffects() }))
    expect(html).toContain('自动码率')
    expect(html).toContain('键鼠特效')
    expect(html).not.toContain('type="checkbox"')
    expect(html).not.toContain('type="number"')
  })
  it('录制范围入口显示选区或显示器', async () => {
    for (const value of ['region', 'display'] as const) {
      const html = await renderToString(h(RecordingSourceSelect, { disabled: false, value }))
      expect(html).toContain(value === 'region' ? '选区' : '显示器')
      expect(html).not.toContain('应用窗口')
    }
  })
})
