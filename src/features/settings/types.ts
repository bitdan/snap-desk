/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Settings } from '@/features/settings/settings'
export type AppDirectory = 'config' | 'data' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard' | 'logs'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
