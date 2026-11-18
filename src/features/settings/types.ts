/** 与原生 camelCase 序列化保持一致的功能通信契约。 */
import type { Settings } from '@/features/settings/settings'
export type AppDirectory = 'config' | 'data' | 'captures' | 'recordings' | 'scrollResults' | 'clipboard' | 'logs'
export interface DirectoryView { kind: AppDirectory; path: string | null; error: string | null }
/** 系统隐私授权状态；supported=false 的平台无需应用内引导。 */
export interface SystemPermissions { supported: boolean; screenCapture: boolean; accessibility: boolean; inputMonitoring: boolean }
export type SystemPermissionKind = 'screenCapture' | 'accessibility' | 'inputMonitoring'
/** 单项功能可用性；reason 是原生中文消息键。 */
export interface DesktopFeature { available: boolean; reason: string | null }
export interface DesktopFeatures {
  screenshot: DesktopFeature; screenshotCursor: DesktopFeature; windowSnap: DesktopFeature; scrollCapture: DesktopFeature
  recording: DesktopFeature; recordingEffects: DesktopFeature; globalShortcuts: DesktopFeature; clipboardHistory: DesktopFeature
  selectedText: DesktopFeature; directPaste: DesktopFeature; pinOnTop: DesktopFeature
}
export interface DesktopProtocols {
  kwinScreenshot: boolean; extImageCopyCapture: boolean; wlrScreencopy: boolean; layerShell: boolean; extDataControl: boolean
  wlrDataControl: boolean; portalScreenshot: boolean; portalScreencast: boolean; portalGlobalShortcuts: boolean; portalRemoteDesktop: boolean
}
/** 会话类型与功能可用性，由原生端在启动时探测（Linux 按实际提供的 Wayland 协议与门户判断）。 */
export interface DesktopSupport { session: 'windows' | 'macos' | 'x11' | 'wayland'; desktop: string; compositor: string; protocols: DesktopProtocols; features: DesktopFeatures; commandLine: string }
export interface AppStatus { version: string; settings: Settings; shortcutError: string | null; unavailableShortcuts: string[]; defaultShortcuts: Settings['shortcuts']; configWarning: string | null; delayRemaining: number; platform: string; desktopSupport: DesktopSupport }
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
