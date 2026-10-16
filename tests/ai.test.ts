import { describe, expect, it } from 'vitest'
import { defaultAiSettings, validateAiSettings } from '../src/lib/ai'
import { defaultSettings } from '../src/lib/settings'

describe('翻译设置契约', () => {
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
})
