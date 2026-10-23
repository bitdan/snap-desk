<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import { recoveryBytes, recoveryItemStatus, recoverySummary, recoveryTime, type RecoveryItem, type RecoveryStatus } from '@/lib/recovery-center'

const emit = defineEmits<{ status: [value: RecoveryStatus]; failure: [message: string] }>()
const status = ref<RecoveryStatus | null>(null), loading = ref(false), busy = ref(false)
const error = ref(''), message = ref(''), confirming = ref<RecoveryItem | null>(null)
const summary = computed(() => status.value ? recoverySummary(status.value.items) : '尚未检查')
async function refresh() {
  if (loading.value) return
  loading.value = true; error.value = ''
  try { status.value = await api.recoveryStatus(); emit('status', status.value); emit('failure', '') }
  catch (e) { error.value = String(e); emit('failure', error.value) }
  finally { loading.value = false }
}
async function openDataDirectory() {
  try { await api.openAppDirectory('data') }
  catch (e) { error.value = String(e) }
}
/** 录屏沿用已有后台保护；其他文件同样不向原生传递页面中的路径。 */
async function act(item: RecoveryItem, action: 'recover' | 'openDirectory' | 'discard') {
  if (busy.value || loading.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    if (item.kind === 'recording') {
      if (action === 'discard') await api.discardRecordingRecovery(item.id)
      else message.value = `已恢复导出：${await api.retryRecordingExport(item.id)}`
    } else {
      const path = await api.recoveryArtifactAction(item.id, action)
      if (path) message.value = `已另存：${path}。原始副本保留，可确认后清理。`
      else if (action === 'openDirectory') message.value = '已打开所在目录。'
    }
    confirming.value = null
    await refresh()
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
onMounted(() => { if (desktop) void refresh() })
</script>

<template>
  <section aria-label="恢复与清理">
    <div class="setting-row gap-3"><span role="status">{{ loading ? '正在检查…' : summary }}</span><Button variant="outline" :disabled="!desktop || loading || busy" @click="refresh">重新检查</Button></div>
    <p v-if="!desktop" class="py-2 text-sm">请在桌面应用中检查残留文件。</p>
    <p v-if="status?.previousStartedAt" class="notice" role="status">上次运行未记录正常退出（启动于 {{ recoveryTime(status.previousStartedAt) }}，进程 {{ status.previousPid }}）。旧截图与采集不会自动恢复；文件由你选择保留、恢复或删除。</p>
    <p v-if="status?.warning" class="notice" role="alert">{{ status.warning }}</p>
    <div v-if="status?.unreadableRecords" class="notice" role="alert"><p>{{ status.unreadableRecords }} 条记录无法安全读取，相关文件已保留；当前列表不代表全部残留。可检查 export-recovery / recording-recovery 中的记录，勿删除仍需保留的源文件。</p><Button variant="outline" @click="openDataDirectory">打开应用数据目录</Button></div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice break-all" role="status">{{ message }}</p>
    <p class="py-2 text-xs text-zinc-500">仅检查本应用登记的录屏、导出和临时文件，不扫描用户目录。未完成编码的文件不会自动修复；已导出的最终文件不会被清理。</p>
    <p v-if="status && !status.items.length && !loading && !status.unreadableRecords" class="py-4 text-sm">当前没有发现已登记的待恢复或可清理文件。</p>
    <div v-for="item in status?.items ?? []" :key="`${item.kind}:${item.id}`" class="border-b py-3">
      <div class="flex flex-wrap items-center justify-between gap-2"><strong class="text-sm">{{ item.title }}</strong><span class="text-xs text-zinc-500">{{ recoveryBytes(item.bytes) }} · {{ recoveryTime(item.modifiedAt) }}</span></div>
      <p class="break-all py-2 text-xs">{{ item.path }}</p>
      <div class="flex flex-wrap items-center justify-between gap-2"><span class="text-xs text-zinc-500">{{ recoveryItemStatus(item) }}</span><div class="flex flex-wrap gap-2">
        <Button v-if="item.ready" variant="outline" :disabled="busy || loading" @click="act(item, 'recover')">{{ item.kind === 'recording' ? '恢复导出' : '另存恢复文件' }}</Button>
        <Button v-if="item.kind !== 'recording' && item.sourceExists" variant="ghost" :disabled="busy || loading" @click="act(item, 'openDirectory')">打开目录</Button>
        <Button variant="ghost" :disabled="busy || loading" @click="confirming = item">清理</Button>
      </div></div>
    </div>
    <div v-if="confirming" class="notice mt-3" role="alertdialog" aria-label="确认清理残留文件">
      <p class="break-all">永久删除“{{ confirming.title }}”的暂存源文件及记录？{{ recoveryBytes(confirming.bytes) }}；已导出的最终文件保留。</p>
      <div class="flex gap-2 pt-2"><Button :disabled="busy || loading" @click="act(confirming, 'discard')">确认清理</Button><Button variant="outline" :disabled="busy" @click="confirming = null">保留</Button></div>
    </div>
  </section>
</template>
