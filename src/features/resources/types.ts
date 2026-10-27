/** 与原生 camelCase 序列化保持一致的功能通信契约。 */

export interface ResourceStatus { directory: string; ocrReady: boolean; modelsReady: boolean; runtimeReady: boolean; runtimeDownloadMib: number; fontReady: boolean }
export interface ResourceProgress { name: string; received: number; total: number }
