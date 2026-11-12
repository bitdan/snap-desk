import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import RecordingSetupMenu from '../../src/features/recording/components/RecordingSetupMenu.vue'
import RecordingSourceSelect from '../../src/features/recording/components/RecordingSourceSelect.vue'
import { defaultInputEffects } from '../../src/features/recording/recording'

describe('紧凑录屏下拉入口', () => {
  it('工具行只放设置和特效图标，当前取值写进提示，不把复选框及自定义输入铺到工具行', async () => {
    const html = await renderToString(h(RecordingSetupMenu, { busy: false, format: 'mp4', fps: 60, quality: 'balanced', encoding: 'auto', mbps: '', countdown: 0, effects: defaultInputEffects() }))
    expect(html).toContain('录屏设置 · MP4 · 60 FPS · 均衡 · 无延时')
    expect(html).toContain('键鼠特效 · 已关闭')
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
