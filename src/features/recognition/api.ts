/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Rect } from '@/shared/geometry/selection'
import type { Annotation } from '@/features/annotations/annotations'
import type { AiSettings, OcrResult, TranslationResult, LocalTranslationStatus } from '@/features/recognition/ai'
import type { QrResult } from '@/features/recognition/types'
import { invoke } from '@/services/ipc'

export const recognitionApi = {
  recognizeText: (id: number, rect: Rect, annotations: Annotation[]) => invoke<string>('recognize_capture_text', { id, rect, annotations }),
  recognizeCapture: (id: number, rect: Rect) => invoke<OcrResult>('recognize_capture_v2', { id, rect }),
  translateText: (requestId: string, text: string, consent: boolean, expectedSettings: AiSettings) => invoke<TranslationResult>('translate_text', { requestId, text, consent, expectedSettings }),
  translationModelStatus: () => invoke<LocalTranslationStatus>('translation_model_status'),
  translationModelAction: (modelId: string, action: 'download' | 'import' | 'delete') => invoke<LocalTranslationStatus>('translation_model_action', { modelId, action }),
  aiTranslationConfig: () => invoke<AiSettings>('ai_translation_config'),
  cancelAiJob: (requestId: string) => invoke<void>('cancel_ai_job', { requestId }),
  aiCredentialSet: (secret: string | null) => invoke<void>('ai_credential_set', { secret }),
  aiProviderStatus: () => invoke<boolean>('ai_provider_status'),
  aiProviderTest: () => invoke<void>('ai_provider_test'),
  copyOcrText: (text: string) => invoke<void>('copy_ocr_text', { text }),
  recognizeQr: (id: number, rect: Rect, annotations: Annotation[]) => invoke<QrResult>('recognize_capture_qr', { id, rect, annotations }),
  recognizePinText: () => invoke<OcrResult>('recognize_pin_text'),
  recognizePinQr: () => invoke<QrResult>('recognize_pin_qr'),
  pinQrAction: (id: number, index: number, action: 'open' | 'copy') => invoke<void>('pin_qr_action', { id, index, action }),
  qrAction: (id: number, index: number, action: 'open' | 'copy') => invoke<void>('qr_action', { id, index, action }),
}
