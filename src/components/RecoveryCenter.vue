<script setup lang="ts">
import { t, te } from '@/lib/i18n'
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
  <section :aria-label='t("恢复与清理")'>
    <div class="setting-row gap-3"><span role="status">{{ loading ? t("正在检查…") : t(summary) }}</span><Button variant="outline" :disabled="!desktop || loading || busy" @click="refresh">{{ t("重新检查") }}</Button></div>
    <p v-if="!desktop" class="py-2 text-sm">{{ t("请在桌面应用中检查残留文件。") }}</p>
    <p v-if="status?.previousStartedAt" class="notice" role="status">{{ t("上次运行未记录正常退出（启动于") }} {{ recoveryTime(status.previousStartedAt) }}{{ t("，进程") }} {{ status.previousPid }}{{ t("）。旧截图与采集不会自动恢复；文件由你选择保留、恢复或删除。") }}</p>
    <p v-if="status?.warning" class="notice" role="alert">{{ te(status.warning) }}</p>
    <div v-if="status?.unreadableRecords" class="notice" role="alert"><p>{{ status.unreadableRecords }} {{ t("条记录读取失败，原文件已保留，请打开数据目录检查。") }}</p><Button variant="outline" @click="openDataDirectory">{{ t("打开应用数据目录") }}</Button></div>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    <p v-if="message" class="notice break-all" role="status">{{ t(message) }}</p>
    <p class="py-2 text-xs text-zinc-500">{{ t("仅清理应用暂存文件，已导出的文件保留。") }}</p>
    <p v-if="status && !status.items.length && !loading && !status.unreadableRecords" class="py-4 text-sm">{{ t("没有待恢复或可清理的文件。") }}</p>
    <div v-for="item in status?.items ?? []" :key="`${item.kind}:${item.id}`" class="border-b py-3">
      <div class="flex flex-wrap items-center justify-between gap-2"><strong class="text-sm">{{ t(item.title) }}</strong><span class="text-xs text-zinc-500">{{ t(recoveryBytes(item.bytes)) }} · {{ recoveryTime(item.modifiedAt) }}</span></div>
      <p class="break-all py-2 text-xs">{{ item.path }}</p>
      <div class="flex flex-wrap items-center justify-between gap-2"><span class="text-xs text-zinc-500">{{ t(recoveryItemStatus(item)) }}</span><div class="flex flex-wrap gap-2">
        <Button v-if="item.ready" variant="outline" :disabled="busy || loading" @click="act(item, 'recover')">{{ item.kind === 'recording' ? t("恢复导出") : t("另存恢复文件") }}</Button>
        <Button v-if="item.kind !== 'recording' && item.sourceExists" variant="ghost" :disabled="busy || loading" @click="act(item, 'openDirectory')">{{ t("打开目录") }}</Button>
        <Button variant="ghost" :disabled="busy || loading" @click="confirming = item">{{ t("清理") }}</Button>
      </div></div>
    </div>
    <div v-if="confirming" class="notice mt-3" role="alertdialog" :aria-label='t("确认清理残留文件")'>
      <p class="break-all">{{ t("永久删除“") }}{{ t(confirming.title) }}{{ t("”的暂存源文件及记录？") }}{{ t(recoveryBytes(confirming.bytes)) }}{{ t("；已导出的最终文件保留。") }}</p>
      <div class="flex gap-2 pt-2"><Button :disabled="busy || loading" @click="act(confirming, 'discard')">{{ t("确认清理") }}</Button><Button variant="outline" :disabled="busy" @click="confirming = null">{{ t("保留") }}</Button></div>
    </div>
  </section>
</template>
