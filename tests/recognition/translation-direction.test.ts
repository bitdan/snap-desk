import { describe, expect, it } from 'vitest'
import { automaticTarget, detectTextLanguage } from '../../src/features/recognition/translation-direction'

describe('中英自动方向', () => {
  it.each([
    ['请检查服务连接是否正常', 'zh-CN', 'en'],
    ['Please check the service connection.', 'en', 'zh-CN'],
    ['请检查 API 的错误信息', 'zh-CN', 'en'],
    ['Please send the report to 张三 today.', 'en', 'zh-CN'],
    ['你好 hello', 'mixed', 'zh-CN'],
    ['你好世界朋友 hello world', 'zh-CN', 'en'],
    ['你好 hello world again', 'en', 'zh-CN'],
    ['请确认 https://example.com/very/long/path 的配置', 'zh-CN', 'en'],
    ['请检查 `request.payload.userId` 的取值', 'zh-CN', 'en'],
    ['请检查\n```js\nconst message = "hello world this is code"\n```\n的取值', 'zh-CN', 'en'],
    ['你好 user@example.com 2026', 'zh-CN', 'en'],
    ['你好 C:\\Users\\Administrator\\Documents\\test.txt', 'zh-CN', 'en'],
    ['12345 -- https://example.com `hello`', 'unknown', 'zh-CN'],
    ['', 'unknown', 'zh-CN'],
  ])('%s', (text, language, target) => {
    expect(detectTextLanguage(text)).toBe(language)
    expect(automaticTarget(detectTextLanguage(text), 'zh-CN')).toBe(target)
  })
  it('比例接近或不含语言时沿用用户默认目标', () => {
    expect(automaticTarget('mixed', 'en')).toBe('en')
    expect(automaticTarget('unknown', 'ja')).toBe('ja')
  })
})
