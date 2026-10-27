import { localizedDate, t } from '@/shared/i18n/i18n'

/** 后台拥有来源路径；所有恢复/清理命令只接收记录 ID。 */
export interface RecoveryItem {
  id: string
  kind: 'recording' | 'export' | 'temporary'
  title: string
  path: string
  bytes: number | null
  modifiedAt: number | null
  ready: boolean
  sourceExists: boolean
  stage: string
}
export interface RecoveryStatus {
  previousStartedAt: number | null
  previousPid: number | null
  warning: string | null
  items: RecoveryItem[]
  unreadableRecords: number
}
export type RecoveryArtifactAction = 'recover' | 'openDirectory' | 'discard'
export function recoveryBytes(bytes: number | null): string {
  if (bytes === null) return t('体积未知')
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1_048_576) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / 1_048_576).toFixed(1)} MiB`
}
export function recoveryTime(milliseconds: number | null): string {
  return milliseconds === null ? t('时间未知') : localizedDate(milliseconds)
}
export function recoverySummary(items: RecoveryItem[]): string {
  const known = items.reduce((sum, item) => sum + (item.bytes ?? 0), 0)
  const unknown = items.filter(item => item.bytes === null).length
  return t('{0} 项 · {1}{2}', { 0: items.length, 1: recoveryBytes(known), 2: unknown ? t('，{0} 项体积未知', { 0: unknown }) : '' })
}
export function recoveryItemStatus(item: RecoveryItem): string {
  if (item.stage === 'unsafe') return '目录内容与登记不一致，需手动检查；自动清理会拒绝操作'
  if (!item.sourceExists) return '源文件缺失，可清理记录'
  if (item.kind === 'temporary') return '临时快照 / 未交付批次，可打开目录检查'
  if (item.ready) return item.stage === 'exported' || item.stage === 'cleanup-pending' ? '导出后残留，可检查或清理' : '已完成编码，可恢复导出'
  return item.kind === 'recording' ? '未确认完成封装，只能保留或清理' : '编码未完成，可打开目录检查；不会自动修复'
}
