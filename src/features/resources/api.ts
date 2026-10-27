/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { ResourceStatus } from '@/features/resources/types'
import { invoke } from '@/services/ipc'

export const resourcesApi = {
  resourceStatus: () => invoke<ResourceStatus>('resource_status'),
  downloadResources: (kind: 'ocr' | 'font') => invoke<ResourceStatus>('download_resources', { kind }),
  chooseExportFont: () => invoke<ResourceStatus>('choose_export_font'),
}
