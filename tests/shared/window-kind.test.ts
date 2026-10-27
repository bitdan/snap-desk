import { describe, expect, it } from 'vitest'
import { resolveWindowKind } from '../../src/shared/window/window-kind'

describe('统一 Vue 窗口入口', () => {
  it('原生标签优先于 URL，控制条和边框重载不会挂成主页', () => {
    for (const name of ['recording-controls', 'capture', 'scroll-preview', 'scroll-controls', 'main']) {
      expect(resolveWindowKind(name, '?window=pin')).toBe(name)
    }
    for (const kind of ['recording', 'scroll']) for (const edge of ['top', 'bottom', 'left', 'right']) {
      expect(resolveWindowKind(`${kind}-edge-${edge}`, '')).toBe('selection-edge')
    }
    expect(resolveWindowKind('pin-123', '')).toBe('pin')
  })
  it('网页预览使用明确参数，缺省为主页', () => {
    expect(resolveWindowKind(null, '?window=recording-controls')).toBe('recording-controls')
    expect(resolveWindowKind(null, '?window=selection-edge')).toBe('selection-edge')
    expect(resolveWindowKind(null, '')).toBe('main')
  })
})
