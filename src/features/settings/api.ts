/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { Settings } from '@/features/settings/settings'
import type { ShortcutProbeReport } from '@/features/settings/shortcut-probe'
import type { AppDirectory, DirectoryView, AppStatus } from '@/features/settings/types'
import { invoke } from '@/services/ipc'

export const settingsApi = {
  autostartEnabled: () => invoke<boolean>('autostart_enabled'),
  setAutostartEnabled: (enabled: boolean) => invoke<boolean>('set_autostart_enabled', { enabled }),
  completeOnboarding: () => invoke<Settings>('complete_onboarding'),
  chooseCaptureDirectory: (reset = false) => invoke<DirectoryView | null>('choose_capture_directory', { reset }),
  appDirectories: () => invoke<DirectoryView[]>('app_directories'),
  openAppDirectory: (kind: AppDirectory) => invoke<void>('open_app_directory', { kind }),
  copyAppDirectory: (kind: AppDirectory) => invoke<void>('copy_app_directory', { kind }),
  copyShortcutExample: () => invoke<void>('copy_shortcut_example'),
  importSettings: () => invoke<Settings | null>('import_settings'),
  exportSettings: () => invoke<boolean>('export_settings'),
  openProjectHomepage: () => invoke<void>('open_project_homepage'),
  status: () => invoke<AppStatus>('app_status'),
  mainWindowReady: () => invoke<void>('main_window_ready'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  setShortcutsEnabled: (enabled: boolean) => invoke<Settings>('set_shortcuts_enabled', { enabled }),
  setShortcutEditing: (editing: boolean) => invoke<void>('set_shortcut_editing', { editing }),
  probeShortcuts: () => invoke<ShortcutProbeReport>('probe_shortcuts'),
  hide: () => invoke<void>('hide_main'),
}
