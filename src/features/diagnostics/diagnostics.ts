/** 与 Rust diagnostics 的受控查询、状态及返回值保持一致。 */
export type LogLevel = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG'
export type LogModule = 'app' | 'capture' | 'scroll' | 'recording' | 'ocr' | 'translation' | 'clipboard' | 'frontend' | 'diagnostics'
export interface LogQuery { limit: number; level: LogLevel | null; module: LogModule | null }
export interface LogEntry { timestamp: string; level: LogLevel; module: LogModule; message: string }
export interface RecentLogs { entries: LogEntry[]; scannedBytes: number; omittedLines: number; truncated: boolean }
export interface DiagnosticsStatus {
  level: 'INFO' | 'DEBUG'
  debugRemainingSeconds: number
  directory: string | null
  error: string | null
  totalBytes: number
  archives: number
  recentErrors: number
  exportSupported: boolean
}
export interface ClearArchivesResult { cancelled: boolean; deleted: number; failed: number }

/** 搜索只匹配后端已经读取并过滤的内容，不触发磁盘扫描或远程请求。 */
export function searchLogs(entries: LogEntry[], search: string): LogEntry[] {
  const term = search.trim().toLowerCase()
  return term ? entries.filter(entry => entry.message.toLowerCase().includes(term)) : entries
}

/** 只用于界面倒计时；实际写盘级别始终由后端单调时钟决定。 */
export function debugSeconds(remaining: number, loadedAt: number, now: number): number {
  return Math.max(0, remaining - Math.floor(Math.max(0, now - loadedAt) / 1000))
}
