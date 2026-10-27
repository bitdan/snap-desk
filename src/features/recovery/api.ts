/** 本功能拥有的 IPC 命令；底层调用统一经过 services/ipc。 */
import type { RecoveryStatus, RecoveryArtifactAction } from '@/features/recovery/recovery-center'
import { invoke } from '@/services/ipc'

export const recoveryApi = {
  recoveryStatus: () => invoke<RecoveryStatus>('recovery_status'),
  recoveryArtifactAction: (id: string, action: RecoveryArtifactAction) => invoke<string | null>('recovery_artifact_action', { id, action }),
}
