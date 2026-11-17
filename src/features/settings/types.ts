/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Settings } from '@/features/settings/settings'
export type AppDirectory = 'config' | 'data' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard' | 'logs'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
/** 系统隐私授权状态；supported=false 的平台无需应用内引导。 */
export interface SystemPermissions { supported: boolean; screenCapture: boolean; accessibility: boolean; inputMonitoring: boolean }
export type SystemPermissionKind = 'screenCapture' | 'accessibility' | 'inputMonitoring'
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; unavailableShortcuts: string[]; defaultShortcuts: Settings['shortcuts']; configWarning: string | null; delayRemaining: number; platform: string }
/** 安装包类型由原生运行标记确定，页面不能指定更新地址。 */
export interface AppUpdateView {
  currentVersion: string
  availableVersion: string | null
  notes: string | null
  packageKind: 'nsis' | 'standalone' | 'app' | 'appimage' | 'unsupported'
  supported: boolean
  configured: boolean
  phase: 'idle' | 'checking' | 'available' | 'latest' | 'downloading' | 'ready' | 'installing' | 'error'
  downloaded: number
  total: number | null
  error: string | null
}
