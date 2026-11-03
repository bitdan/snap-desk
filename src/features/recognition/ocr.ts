/** OCR 引擎与识别语言独立于文本翻译配置；未选择的引擎不会被调用。 */
export interface OcrSettings { provider: 'local-ppocr' | 'windows-media' | 'service'; language: string; serviceId?: string }
export interface OcrLanguage { tag: string; name: string }
export interface SystemOcrStatus { supported: boolean; languages: OcrLanguage[]; error: string | null }
export function defaultOcrSettings(): OcrSettings { return { provider: 'local-ppocr', language: 'auto' } }
