/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Settings } from '@/features/settings/settings'
export type AppDirectory = 'config' | 'data' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard' | 'logs'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; configWarning: string | null; delayRemaining: number; platform: string }
/** 安装包类型由原生运行标记确定，页面不能指定更新地址。 */
export interface AppUpdateView {
  currentVersion: string
  availableVersion: string | null
  notes: string | null
  packageKind: 'nsis' | 'standalone' | 'unsupported'
  supported: boolean
  configured: boolean
  phase: 'idle' | 'checking' | 'available' | 'latest' | 'downloading' | 'ready' | 'installing' | 'error'
  downloaded: number
  total: number | null
  error: string | null
}
