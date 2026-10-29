/** 稳定的桌面 API 汇总入口；命令、事件与类型由所属功能维护，此处只做组合。 */
import { diagnosticsApi } from '@/features/diagnostics/api'
import { resourcesApi } from '@/features/resources/api'
import { recognitionApi } from '@/features/recognition/api'
import { clipboardApi } from '@/features/clipboard/api'
import { recordingApi } from '@/features/recording/api'
import { settingsApi } from '@/features/settings/api'
import { captureApi } from '@/features/capture/api'
import { pinsApi } from '@/features/pins/api'
import { scrollApi } from '@/features/scroll/api'
export { desktop } from './ipc'
export type { Settings } from '@/features/settings/settings'
export type { CaptureTarget, CaptureDisplay, CaptureFrame, ColorFormat } from '@/features/capture/types'
export type { PinView, PinFrame, PinSummary, PinTransform, PinAction, PinManageAction } from '@/features/pins/types'
export type { ScrollBackground, ScrollView } from '@/features/scroll/types'
export type { QrResult } from '@/features/recognition/types'
export type { ResourceStatus, ResourceProgress } from '@/features/resources/types'
export type { AppDirectory, DirectoryView, AppStatus } from '@/features/settings/types'
export * from '@/features/capture/events'
export * from '@/features/recording/events'
export * from '@/features/resources/events'
export * from '@/features/pins/events'
export * from '@/features/settings/events'
export * from '@/features/scroll/events'
export * from '@/features/clipboard/events'

export const api = {
  ...diagnosticsApi,
  ...resourcesApi,
  ...recognitionApi,
  ...clipboardApi,
  ...recordingApi,
  ...settingsApi,
  ...captureApi,
  ...pinsApi,
  ...scrollApi,
}
