import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import RecordingSetupMenu from '../src/components/RecordingSetupMenu.vue'
import RecordingSourceSelect from '../src/components/RecordingSourceSelect.vue'
import { defaultInputEffects } from '../src/lib/recording'

describe('紧凑录屏下拉入口', () => {
  it('默认只显示当前值和特效入口，不把复选框及自定义输入铺到工具行', async () => {
    const html = await renderToString(h(RecordingSetupMenu, { busy: false, format: 'mp4', quality: 'balanced', encoding: 'auto', mbps: '', countdown: 0, effects: defaultInputEffects() }))
    expect(html).toContain('自动码率')
    expect(html).toContain('键鼠特效')
    expect(html).not.toContain('type="checkbox"')
    expect(html).not.toContain('type="number"')
  })
  it('选中应用时显示其真实图标和名称，缺图标时仍可选', async () => {
    const windows = [{ index: 2, label: 'Weixin · 微信', icon: 'png' }]
    const html = await renderToString(h(RecordingSourceSelect, { disabled: false, value: 'window:2', windows }))
    expect(html).toContain('data:image/png;base64,png')
    expect(html).toContain('Weixin · 微信')
    const fallback = await renderToString(h(RecordingSourceSelect, { disabled: false, value: 'window:2', windows: [{ index: 2, label: '微信' }] }))
    expect(fallback).toContain('微信')
    expect(fallback).not.toContain('data:image/png')
  })
})
