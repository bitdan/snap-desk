import { describe, expect, it } from 'vitest'
import { deepLEndpoints, defaultAiSettings, onlineTranslationReady, validateAiSettings } from '../../src/features/recognition/ai'
import { defaultSettings } from '../../src/features/settings/settings'

describe('翻译设置契约', () => {
  it('DeepL 仅允许官方地址，无需模型，目标语言与在线服务一致', () => {
    for (const endpoint of deepLEndpoints) {
      const settings = { ...defaultAiSettings(), provider: 'deepl' as const, endpoint, model: '', targetLanguage: 'ja' }
      expect(validateAiSettings(settings)).toBeNull()
      expect(onlineTranslationReady(settings)).toBe(true)
    }
    for (const endpoint of ['https://api.deepl.com.evil.test/v2/translate', 'http://127.0.0.1/v2/translate', 'https://api.deepl.com/v2/translate?key=x', 'https://api.deepl.com/v2/usage']) {
      expect(validateAiSettings({ ...defaultAiSettings(), provider: 'deepl', endpoint })).not.toBeNull()
    }
    expect(onlineTranslationReady({ ...defaultAiSettings(), provider: 'online-llm', endpoint: deepLEndpoints[0]! })).toBe(false)
  })
  it('默认不开启网络服务，嵌套草稿互不影响', () => {
    const settings = defaultSettings()
    expect(settings.ai.endpoint).toBe('')
    expect(settings.ai.provider).toBe('local-ctranslate2')
    expect(validateAiSettings(settings.ai)).toBeNull()
    settings.ai.targetLanguage = 'en'
    expect(defaultSettings().ai.targetLanguage).toBe('zh-CN')
    expect(Object.keys(settings.ai)).not.toContain('apiKey')
  })
  it('本地只允许双向中英，在线保留其他目标语言', () => {
    for (const targetLanguage of ['zh-CN', 'en']) {
      expect(validateAiSettings({ ...defaultAiSettings(), targetLanguage })).toBeNull()
    }
    for (const targetLanguage of ['ja', 'ko', 'fr', 'de']) {
      expect(validateAiSettings({ ...defaultAiSettings(), targetLanguage })).not.toBeNull()
      expect(validateAiSettings({ ...defaultAiSettings(), provider: 'online-llm', targetLanguage })).toBeNull()
    }
  })
  it('拒绝不安全地址、URL 凭证和非本机明文连接', () => {
    for (const endpoint of ['http://example.com/v1', 'https://key@example.com/v1', 'https://example.com/?key=x', 'file:///tmp/test', 'https://example.com/#secret', 'invalid']) {
      expect(validateAiSettings({ ...defaultAiSettings(), endpoint })).not.toBeNull()
    }
    for (const endpoint of ['https://example.com/v1/chat/completions', 'http://127.0.0.1:8080/v1/chat/completions']) {
      expect(validateAiSettings({ ...defaultAiSettings(), endpoint })).toBeNull()
    }
  })
  it('限制超时、目标语言与模型长度', () => {
    for (const timeoutSeconds of [0, 4, 121, NaN, 5.5]) {
      expect(validateAiSettings({ ...defaultAiSettings(), timeoutSeconds })).not.toBeNull()
    }
    expect(validateAiSettings({ ...defaultAiSettings(), targetLanguage: 'invalid' })).not.toBeNull()
    expect(validateAiSettings({ ...defaultAiSettings(), model: 'x\nAuthorization' })).not.toBeNull()
    expect(validateAiSettings({ ...defaultAiSettings(), model: 'x'.repeat(201) })).not.toBeNull()
  })
  it('闲置回收默认五分钟，允许关闭且拒绝非法时长', () => {
    expect(defaultAiSettings().localIdleSeconds).toBe(300)
    for (const localIdleSeconds of [0, 30, 300, 3600]) {
      expect(validateAiSettings({ ...defaultAiSettings(), localIdleSeconds })).toBeNull()
    }
    for (const localIdleSeconds of [-1, 1, 29, 3601, NaN, Infinity, 30.5]) {
      expect(validateAiSettings({ ...defaultAiSettings(), localIdleSeconds })).not.toBeNull()
    }
  })
})
