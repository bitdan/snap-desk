import { describe, expect, it } from 'vitest'
import { recordingWindowOptions } from '../src/lib/recording'

describe('应用窗口录制来源', () => {
  it('保留暂时无屏幕坐标的最小化或隐藏应用，候选编号不压缩', () => {
    const options = recordingWindowOptions([
      { rect: { x: 0, y: 0, width: 0, height: 0 }, kind: 'window', windowIndex: 3, appName: 'Weixin', title: '微信', recordingOnly: true },
      { rect: { x: 0, y: 0, width: 600, height: 400 }, kind: 'window', windowIndex: 4, title: '编辑器' },
    ])
    expect(options).toEqual([{ index: 0, label: 'Weixin · 微信' }, { index: 1, label: '编辑器' }])
  })
  it('过滤子控件并保留原候选下标，应用重名时用标题区分', () => {
    const rect = { x: 0, y: 0, width: 200, height: 100 }
    const options = recordingWindowOptions([
      { rect, kind: 'window', windowIndex: 0, appName: 'Editor', title: 'first.ts' },
      { rect, kind: 'element', windowIndex: 0 },
      { rect, kind: 'window', windowIndex: 1, appName: 'Editor', title: 'second.ts' },
      { rect, kind: 'window', windowIndex: 2, title: 'Terminal' },
    ])
    expect(options).toEqual([{ index: 0, label: 'Editor · first.ts' }, { index: 2, label: 'Editor · second.ts' }, { index: 3, label: 'Terminal' }])
    expect(recordingWindowOptions([])).toEqual([])
  })
})
