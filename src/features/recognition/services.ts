/** 连接描述不含密钥；协议决定可用能力，模型和账号可以分别建立连接。 */
import { translationPresets, type AiSettings } from './ai'
export type ServiceProtocol = 'openai-compatible' | 'ocr-space' | 'paddle-ocr' | 'deepl'
export type ConnectionMode = 'offline' | 'online' | 'local-api'
/** 本机 API 仅代表请求发往回环地址，不能据此保证服务端不再联网。 */
export function connectionMode(endpoint: string): ConnectionMode | undefined {
  try {
    const url = new URL(endpoint)
    if (!['http:', 'https:'].includes(url.protocol)) return undefined
    return ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname) ? 'local-api' : 'online'
  } catch { return undefined }
}
/** 云端优先展示；保持同类原有顺序，不改变用户当前选择。 */
export function cloudFirst<T extends { connection?: ConnectionMode; sortPriority?: number }>(items: T[]): T[] {
  const rank = (mode?: ConnectionMode) => mode === 'online' ? 0 : mode === 'local-api' ? 2 : mode === 'offline' ? 3 : 1
  return [...items].sort((left, right) => (left.sortPriority ?? rank(left.connection)) - (right.sortPriority ?? rank(right.connection)))
}
export interface RecognitionService { id: string; name: string; protocol: ServiceProtocol; endpoint: string; model: string; vision: boolean }
export const paddleJobsEndpoint = 'https://paddleocr.aistudio-app.com/api/v2/ocr/jobs'
export const paddleModels = ['PP-OCRv6', 'PaddleOCR-VL-1.6', 'PP-StructureV3', 'PP-OCRv5'] as const
export function isPaddleJobsEndpoint(endpoint: string): boolean {
  try { return new URL(endpoint).pathname.replace(/\/+$/, '') === '/api/v2/ocr/jobs' } catch { return false }
}
export const servicePresets = [
  { id: 'groq', name: 'Groq', protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'openai/gpt-oss-120b', vision: false, portal: 'https://console.groq.com/keys' },
  { id: 'groq-vision', name: 'Groq Vision', protocol: 'openai-compatible', endpoint: 'https://api.groq.com/openai/v1/chat/completions', model: 'qwen/qwen3.8-27b', vision: true, portal: 'https://console.groq.com/keys' },
  { id: 'ocr-space', name: 'OCR.space', protocol: 'ocr-space', endpoint: 'https://api.ocr.space/parse/image', model: '2', vision: true, portal: 'https://ocr.space/ocrapi' },
  { id: 'paddle-ocr', name: 'PaddleOCR API', protocol: 'paddle-ocr', endpoint: paddleJobsEndpoint, model: 'PP-OCRv6', vision: true, portal: 'https://aistudio.baidu.com/paddleocr/task' },
  { id: 'deepl', name: 'DeepL API Free', protocol: 'deepl', endpoint: 'https://api-free.deepl.com/v2/translate', model: '', vision: false, portal: 'https://www.deepl.com/your-account/keys' },
  { id: 'custom', name: '自定义兼容服务', protocol: 'openai-compatible', endpoint: '', model: '', vision: false, portal: '' },
] as const
/** 用协议、地址和图片能力归属预设，显示名称可由用户改名，不能拿名称去重。 */
export function servicePresetId(service: RecognitionService): string {
  if (service.protocol !== 'openai-compatible') return service.protocol
  return servicePresets.find(item => item.protocol === service.protocol && item.endpoint === service.endpoint && item.vision === service.vision)?.id ?? 'custom'
}
export function translationPresetId(service: RecognitionService): string {
  return service.protocol === 'deepl' ? 'deepl' : translationPresets.find(item => item.endpoint === service.endpoint)?.value ?? 'custom'
}
/** 显示能力与部署位置，避免把本机兼容模型误标为云端服务。 */
export function serviceKind(service: RecognitionService): string {
  if (service.protocol !== 'openai-compatible') return service.protocol === 'deepl' ? '在线翻译' : '在线 OCR'
  try { if (['localhost', '127.0.0.1', '[::1]'].includes(new URL(service.endpoint).hostname)) return '本机 AI' } catch { /* 无效地址由表单校验报告。 */ }
  return service.vision ? 'AI 视觉 / 翻译' : 'AI 翻译'
}
export function supportsOcr(service: RecognitionService): boolean { return service.protocol === 'ocr-space' || service.protocol === 'paddle-ocr' || (service.protocol === 'openai-compatible' && service.vision) }
export function supportsTranslation(service: RecognitionService): boolean { return ['openai-compatible', 'deepl'].includes(service.protocol) }
export function translationService(settings: AiSettings, service: RecognitionService): AiSettings {
  return { ...settings, serviceId: service.id, provider: service.protocol === 'deepl' ? 'deepl' : 'online-llm', endpoint: service.endpoint, model: service.model }
}
export function validateService(value: RecognitionService): string | null {
  if (!/^[a-zA-Z0-9-]{1,80}$/.test(value.id) || !value.name.trim() || [...value.name].length > 80 || /\p{Cc}/u.test(value.name)) return '服务名称或标识无效。'
  if (!['openai-compatible', 'ocr-space', 'paddle-ocr', 'deepl'].includes(value.protocol)) return '不支持此服务协议。'
  try {
    const url = new URL(value.endpoint)
    if (value.endpoint.length > 2048 || url.username || url.password || url.search || url.hash || !(url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)))) return '服务地址必须使用 HTTPS；HTTP 仅限本机，不能含凭证或查询参数。'
  } catch { return '请填写完整接口地址。' }
  if (new TextEncoder().encode(value.model).length > 200 || /\p{Cc}/u.test(value.model) || (value.protocol === 'openai-compatible' && !value.model.trim())) return '请填写有效的模型 ID。'
  if (value.protocol === 'ocr-space' && !['1', '2', '3'].includes(value.model)) return 'OCR.space 引擎只能为 1、2 或 3。'
  if (value.protocol === 'paddle-ocr' && !(isPaddleJobsEndpoint(value.endpoint) ? [...paddleModels] : ['PP-OCRv5', 'PP-OCRv6']).includes(value.model)) return '所选 PaddleOCR 接口不支持此模型。'
  if (value.protocol === 'deepl' && !['https://api-free.deepl.com/v2/translate', 'https://api.deepl.com/v2/translate'].includes(value.endpoint)) return 'DeepL 仅支持官方翻译接口。'
  return null
}
