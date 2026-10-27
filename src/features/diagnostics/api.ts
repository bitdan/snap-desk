/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { DiagnosticsStatus, LogQuery, RecentLogs, ClearArchivesResult } from '@/features/diagnostics/diagnostics'
import { invoke } from '@/services/ipc'

export const diagnosticsApi = {
  diagnosticsStatus: () => invoke<DiagnosticsStatus>('diagnostics_status'),
  diagnosticsRecentLogs: (query: LogQuery) => invoke<RecentLogs>('diagnostics_recent_logs', { query }),
  diagnosticsExport: () => invoke<boolean>('diagnostics_export'),
  diagnosticsClearArchives: () => invoke<ClearArchivesResult>('diagnostics_clear_archives'),
  diagnosticsSetDebug: (enabled: boolean) => invoke<DiagnosticsStatus>('diagnostics_set_debug', { enabled }),
}
