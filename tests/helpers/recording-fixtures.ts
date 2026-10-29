import type { RecordingAnnotationTools } from '../../src/features/recording/recording-annotations'

/** 模拟原生返回的工具快照；生产界面直接消费真实画布状态，不再维护重复默认值。 */
export const defaultRecordingAnnotationTools = (): RecordingAnnotationTools => ({
  tool: 'pen', color: '#ef4444', width: 3, opacity: 1, fontSize: 24,
  fontFamily: 'sans', magnification: 2, lifetimeMs: 0, fadeOutMs: 500,
})
