import { describe, expect, it } from 'vitest'
import { shortcutKeys, shortcutText } from '@/shared/ui/shortcut-label'

describe('shortcut labels', () => {
  it('uses macOS modifier symbols in the system order', () => {
    expect(shortcutKeys('Ctrl+Alt+Shift+Super+A', true)).toEqual(['⌃', '⌥', '⇧', '⌘', 'A'])
    expect(shortcutText('Alt+V', true)).toBe('⌥V')
    expect(shortcutText('Ctrl + Shift', true)).toBe('⌃⇧')
    expect(shortcutText('F1', true)).toBe('F1')
  })

  it('leaves other platforms unchanged', () => {
    expect(shortcutKeys('Ctrl+Shift+A', false)).toEqual(['Ctrl', 'Shift', 'A'])
    expect(shortcutText('Ctrl + Shift', false)).toBe('Ctrl + Shift')
  })
})
