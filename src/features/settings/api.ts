/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Settings } from '@/features/settings/settings'
import type { ShortcutProbeReport } from '@/features/settings/shortcut-probe'
import type { AppDirectory, DirectoryView, AppStatus, AppUpdateView, SystemPermissionKind, SystemPermissions } from '@/features/settings/types'
import { invoke } from '@/services/ipc'
import { updatePreview, previewUpdater } from './update-preview'

export const settingsApi = {
  appUpdateStatus: () => updatePreview ? previewUpdater.status() : invoke<AppUpdateView>('app_update_status'),
  checkAppUpdate: () => updatePreview ? previewUpdater.check() : invoke<AppUpdateView>('check_app_update'),
  downloadAppUpdate: () => updatePreview ? previewUpdater.download() : invoke<AppUpdateView>('download_app_update'),
  installAppUpdate: () => updatePreview ? previewUpdater.install() : invoke<void>('install_app_update'),
  /** 自动下载失败时在浏览器中打开候选版本的发布页；地址由原生端决定。 */
  openAppUpdatePage: () => invoke<void>('open_app_update_page'),
  autostartEnabled: () => invoke<boolean>('autostart_enabled'),
  setAutostartEnabled: (enabled: boolean) => invoke<boolean>('set_autostart_enabled', { enabled }),
  completeOnboarding: () => invoke<Settings>('complete_onboarding'),
  chooseCaptureDirectory: (reset = false) => invoke<DirectoryView | null>('choose_capture_directory', { reset }),
  appDirectories: () => invoke<DirectoryView[]>('app_directories'),
  openAppDirectory: (kind: AppDirectory) => invoke<void>('open_app_directory', { kind }),
  copyAppDirectory: (kind: AppDirectory) => invoke<void>('copy_app_directory', { kind }),
  copyShortcutExample: () => invoke<void>('copy_shortcut_example'),
  importSettings: () => invoke<Settings | null>('import_settings'),
  exportSettings: (includeSecrets = false) => invoke<boolean>('export_settings', { includeSecrets }),
  openProjectHomepage: () => invoke<void>('open_project_homepage'),
  status: () => invoke<AppStatus>('app_status'),
  mainWindowReady: () => invoke<void>('main_window_ready'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  setShortcutsEnabled: (enabled: boolean) => invoke<Settings>('set_shortcuts_enabled', { enabled }),
  setShortcutEditing: (editing: boolean) => invoke<void>('set_shortcut_editing', { editing }),
  probeShortcuts: () => invoke<ShortcutProbeReport>('probe_shortcuts'),
  systemPermissions: () => invoke<SystemPermissions>('system_permissions'),
  requestSystemPermission: (kind: SystemPermissionKind) => invoke<SystemPermissions>('request_system_permission', { kind }),
  hide: () => invoke<void>('hide_main'),
}
