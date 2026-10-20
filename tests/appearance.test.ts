import { afterEach, describe, expect, it, vi } from 'vitest'
import { appearance, applyAppearance, initializeAppearance, parseAppearance } from '../src/lib/appearance'
import { t } from '../src/lib/i18n'
import { defaultSettings } from '../src/lib/settings'

afterEach(() => { appearance.value = { theme: 'system', language: 'zh-CN' }; vi.unstubAllGlobals() })

describe('appearance preferences', () => {
  it('rejects invalid cached preferences without trusting arbitrary strings', () => {
    for (const value of [null, {}, { theme: 'black', language: 'en' }, { theme: 'dark', language: 'fr' }])
      expect(parseAppearance(value)).toBeNull()
    expect(parseAppearance({ theme: 'dark', language: 'en' })).toEqual({ theme: 'dark', language: 'en' })
    const a = defaultSettings(), b = defaultSettings()
    a.appearance.theme = 'dark'
    expect(b.appearance.theme).toBe('system')
  })
  it('follows the OS, persists explicit changes, and synchronizes other windows', () => {
    const root = { dataset: {} as Record<string, string>, lang: '' }
    const media = { matches: true, addEventListener: vi.fn() }
    const storage = { getItem: vi.fn(() => '{broken'), setItem: vi.fn() }
    const listener = vi.fn()
    vi.stubGlobal('document', { documentElement: root })
    vi.stubGlobal('matchMedia', () => media)
    vi.stubGlobal('localStorage', storage)
    vi.stubGlobal('window', { addEventListener: listener })
    initializeAppearance()
    expect(root.dataset.theme).toBe('dark')
    media.matches = false
    media.addEventListener.mock.calls[0][1]()
    expect(root.dataset.theme).toBe('light')
    applyAppearance({ theme: 'light', language: 'en' })
    expect(root.dataset.theme).toBe('light')
    expect(root.lang).toBe('en')
    expect(storage.setItem).toHaveBeenCalledWith('snap-desk-appearance', '{"theme":"light","language":"en"}')
    const savedReference = appearance.value
    applyAppearance({ theme: 'light', language: 'en' })
    expect(appearance.value).toBe(savedReference)
    media.matches = true
    media.addEventListener.mock.calls[0][1]()
    expect(root.dataset.theme).toBe('light')
    listener.mock.calls[0][1]({ key: 'snap-desk-appearance', newValue: '{"theme":"dark","language":"zh-CN"}' })
    expect(root.dataset.theme).toBe('dark')
    expect(root.lang).toBe('zh-CN')
  })
  it('switches known messages and preserves user or native error text', () => {
    expect(t('截图')).toBe('截图')
    appearance.value.language = 'en'
    expect(t('截图')).toBe('Capture')
    expect(t('unknown OS error')).toBe('unknown OS error')
  })
})
