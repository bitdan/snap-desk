/** 与 Rust recognition 模块一致。坐标相对于裁剪原图，不是屏幕/CSS 像素。 */
export interface PolygonPoint { x: number; y: number }
export interface OcrLine {
  id: string
  text: string
  confidence: number | null
  polygon: PolygonPoint[] | null
  language: string | null
  order: number
  paragraphId: string | null
}
export interface OcrResult {
  requestId: string
  provider: string
  imageWidth: number
  imageHeight: number
  fullText: string
  lines: OcrLine[]
  elapsedMs: number
  warnings: string[]
}
/** 默认本地 OCR + 离线翻译；在线服务需要显式选择和文本上传确认。 */
export interface AiSettings { provider: 'local-ctranslate2' | 'online-llm' | 'deepl'; endpoint: string; model: string; targetLanguage: string; timeoutSeconds: number; localIdleSeconds: number }
export const targetLanguages = [
  { value: 'zh-CN', label: '简体中文' }, { value: 'en', label: '英语' },
  { value: 'ja', label: '日语' }, { value: 'ko', label: '韩语' },
  { value: 'fr', label: '法语' }, { value: 'de', label: '德语' },
]
export function defaultAiSettings(): AiSettings {
  return { provider: 'local-ctranslate2', endpoint: '', model: '', targetLanguage: 'zh-CN', timeoutSeconds: 45, localIdleSeconds: 300 }
}
export function validateAiSettings(value: AiSettings): string | null {
  if (!['local-ctranslate2', 'online-llm', 'deepl'].includes(value.provider)) return '不支持此翻译引擎。'
  if (value.provider === 'local-ctranslate2' && !['zh-CN', 'en'].includes(value.targetLanguage)) return '本地翻译仅支持英译中和中译英。'
  if (value.endpoint) {
    try {
      const url = new URL(value.endpoint)
      const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
      if (value.endpoint.length > 2048 || url.username || url.password || url.search || url.hash
        || !(url.protocol === 'https:' || (url.protocol === 'http:' && local))) return '请使用 HTTPS 完整接口地址；HTTP 仅限本机，地址不能含凭证或查询参数。'
    } catch { return '服务地址无效。' }
  }
  if (value.provider === 'deepl' && value.endpoint && !deepLEndpoints.includes(value.endpoint)) return 'DeepL 仅支持官方 Free 或 Pro 的 /v2/translate 地址。'
  if (value.model.length > 200 || /\p{Cc}/u.test(value.model)) return '模型名称无效。'
  if (!targetLanguages.some(item => item.value === value.targetLanguage)) return '不支持该目标语言。'
  if (!Number.isInteger(value.timeoutSeconds) || value.timeoutSeconds < 5 || value.timeoutSeconds > 120) return '超时必须为 5～120 秒。'
  if (!Number.isInteger(value.localIdleSeconds) || (value.localIdleSeconds !== 0 && (value.localIdleSeconds < 30 || value.localIdleSeconds > 3600))) return '闲置回收必须为 0 或 30～3600 秒。'
  return null
}
export interface TranslationResult { requestId: string; translatedText: string; targetLanguage: string }

export interface TranslationModelStatus { modelId: string; ready: boolean; downloadable: boolean; revision: string | null; downloadBytes: number }
export interface LocalTranslationStatus { runtimeReady: boolean; runtimeSupported: boolean; models: TranslationModelStatus[] }

/** DeepL 地址只能从官方 Free/Pro 选择；自定义服务仅用于兼容协议。 */
export const deepLEndpoints = ['https://api-free.deepl.com/v2/translate', 'https://api.deepl.com/v2/translate']
export const translationPresets = [
  { value: 'openai', label: 'OpenAI', endpoint: 'https://api.openai.com/v1/chat/completions', model: '' },
  { value: 'local', label: '本机兼容服务', endpoint: 'http://127.0.0.1:11434/v1/chat/completions', model: '' },
]
export function onlineTranslationReady(value: AiSettings): boolean {
  return !!value.endpoint && (value.provider === 'deepl' || !!value.model.trim())
}
export interface TranslationRequest { texts: string[]; sourceLanguage: string | null; targetLanguage: string }
export interface TranslationItem { sourceText: string; translatedText: string }
export interface TranslationBatchResult { requestId: string; translations: TranslationItem[]; targetLanguage: string }
