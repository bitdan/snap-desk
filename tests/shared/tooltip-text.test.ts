import { describe, expect, it } from 'vitest'
import { splitTooltip } from '../../src/shared/ui/tooltip-text'

describe('提示文案拆分快捷键', () => {
  it('拆出末尾的快捷键与括注按键', () => {
    expect(splitTooltip('复制 Ctrl/⌘+C')).toEqual({ label: '复制', keys: ['Ctrl/⌘', 'C'] })
    expect(splitTooltip('重做 Ctrl/⌘+Shift+Z')).toEqual({ label: '重做', keys: ['Ctrl/⌘', 'Shift', 'Z'] })
    expect(splitTooltip('关闭 Esc')).toEqual({ label: '关闭', keys: ['Esc'] })
    expect(splitTooltip('收起工具（Tab）')).toEqual({ label: '收起工具', keys: ['Tab'] })
    expect(splitTooltip('上一张截图（,）')).toEqual({ label: '上一张截图', keys: [','] })
    expect(splitTooltip('Copy Ctrl/⌘+C')).toEqual({ label: 'Copy', keys: ['Ctrl/⌘', 'C'] })
  })
  it('普通说明与中文括注保持原样', () => {
    expect(splitTooltip('矩形（拖动排序，Alt+←/→调整顺序）').keys).toEqual([])
    expect(splitTooltip('恢复 100% 查看比例')).toEqual({ label: '恢复 100% 查看比例', keys: [] })
    expect(splitTooltip('Ctrl＋左键拖出内容；拖入微信前可松开 Ctrl').keys).toEqual(['Ctrl'])
    expect(splitTooltip('置顶开关')).toEqual({ label: '置顶开关', keys: [] })
  })
})
