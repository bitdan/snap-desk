<script setup lang="ts">
import { useRecordingPolling } from '@/features/recording/use-recording-polling'
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, onUnmounted, ref } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'
import type { RecoverableRecording, RecordingResult, RecordingResultAction } from '@/features/recording/recording'
import { encoderLabels, recordingAudioStatus, recordingRecoveryStatus, recordingStats, newerRecording, recordingActive, recordingStatus, recordingTime, type RecorderCapabilities, type RecordingAction, type RecordingDirectory, type RecordingView } from '@/features/recording/recording'

const view = ref<RecordingView | null>(null)
const error = ref('')
const busy = ref(false)
const capabilities = ref<RecorderCapabilities | null>(null)
const directory = ref<RecordingDirectory | null>(null)
const changingDirectory = ref(false)
const recoveries = ref<RecoverableRecording[]>([])
const recoveredPath = ref('')
const discardId = ref<string | null>(null)
async function discardRecovery(id: string) {
  if (busy.value || changingDirectory.value || recordingActive(view.value)) return
  busy.value = true; error.value = ''
  try { await api.discardRecordingRecovery(id); discardId.value = null; await loadRecoveries() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
const result = ref<RecordingResult | null>(null)
const resultBusy = ref(false)
const resultMessage = ref('')
async function loadResult() {
  const next = await api.latestRecordingResult()
  if (!disposed) {
    // 轮询与恢复导出可能同时查询，旧快照不能覆盖更新的成功结果。
    if (result.value && (!next || next.id < result.value.id)) return
    if (result.value?.id !== next?.id) resultMessage.value = ''
    result.value = next
  }
}
async function resultAction(action: RecordingResultAction) {
  if (!result.value || resultBusy.value) return
  const id = result.value.id
  resultBusy.value = true; error.value = ''; resultMessage.value = ''
  try {
    await api.recordingResultAction(id, action)
    if (result.value?.id === id) resultMessage.value = action === 'copyPath' ? '路径已复制。' : '已打开保存目录。'
  } catch (e) { error.value = String(e) }
  finally { resultBusy.value = false }
}
async function loadRecoveries() {
  try { const items = await api.recoverableRecordings(); if (!disposed) recoveries.value = items }
  catch (e) { if (!disposed) error.value = String(e) }
}
async function retryExport(id: string) {
  if (busy.value || changingDirectory.value || recordingActive(view.value)) return
  busy.value = true; error.value = ''; recoveredPath.value = ''
  try { recoveredPath.value = await api.retryRecordingExport(id); await loadRecoveries(); await loadResult() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
let disposed = false
async function refresh() {
  try {
    const next = await api.currentRecording()
    if (!disposed) {
      const ended = recordingActive(view.value) && !recordingActive(next)
      view.value = newerRecording(view.value, next)
      if (!recordingActive(view.value)) await loadResult()
      if (ended) await loadRecoveries()
    }
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
useRecordingPolling(refresh, () => view.value?.phase, e => { if (!disposed) error.value = String(e) }, { enabled: () => desktop })
async function control(action: RecordingAction) {
  if (!view.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action); await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function select() {
  busy.value = true; error.value = ''
  try { await api.selectRecordingArea() } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function chooseDirectory(reset = false) {
  if (changingDirectory.value || busy.value || recordingActive(view.value)) return
  changingDirectory.value = true; error.value = ''
  try {
    const next = await api.chooseRecordingDirectory(reset)
    if (next && !disposed) directory.value = next
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { changingDirectory.value = false }
}
onMounted(() => {
  if (!desktop) return
  void loadRecoveries()
  void api.currentRecordingDirectory().then(value => { if (!disposed) directory.value = value })
    .catch(e => { if (!disposed) error.value = String(e) })
  void api.recordingCapabilities().then(value => { if (!disposed) capabilities.value = value })
    .catch(e => { if (!disposed) error.value = String(e) })
})
onUnmounted(() => { disposed = true })
</script>

<template>
  <section>
    <div v-if="error" class="notice error" role="alert">{{ te(error) }}</div>
    <p v-if="recoveredPath" class="notice break-all" role="status">{{ t("已重新导出：") }}{{ recoveredPath }}</p>
    <div v-if="result" class="settings-card mb-3">
      <div class="setting-row"><strong>{{ t("最近完成的录屏") }}</strong><span class="text-sm text-zinc-500">{{ t("本次运行") }}</span></div>
      <p class="py-2 break-all font-medium">{{ result.filename }}</p>
      <p class="break-all text-xs text-zinc-500">{{ result.path }}</p>
      <div class="flex flex-wrap gap-2 py-4">
        <Button variant="outline" :disabled="resultBusy" @click="resultAction('openDirectory')">{{ t("打开目录") }}</Button>
        <Button variant="ghost" :disabled="resultBusy" @click="resultAction('copyPath')">{{ t("复制路径") }}</Button>
      </div>
      <p v-if="resultMessage" class="text-sm pb-3" role="status">{{ t(resultMessage) }}</p>
    </div>
    <div class="mb-3">
      <div class="setting-row"><strong>{{ t("可恢复录制 / 清理残留（") }}{{ recoveries.length }}）</strong><Button variant="outline" :disabled="busy" @click="loadRecoveries">{{ t("刷新") }}</Button></div>
      <p class="py-2 text-sm">{{ recoveries.length ? t("仅完整录制文件可重新导出。") : t("暂无待恢复录制。") }}</p>
      <div v-for="item in recoveries" :key="item.id" class="setting-row gap-3">
        <span class="min-w-0 flex-1 break-all">{{ item.path }}<small>{{ t(recordingRecoveryStatus(item)) }}</small></span>
        <div class="flex flex-wrap gap-2">
          <Button v-if="item.finalized && item.sourceExists" :disabled="busy || changingDirectory || recordingActive(view)" @click="retryExport(item.id)">{{ item.exported ? t("检查导出") : t("重试导出") }}</Button>
          <Button variant="ghost" :disabled="busy || changingDirectory || recordingActive(view)" @click="discardId = item.id">{{ t("清理残留") }}</Button>
        </div>
      </div>
      <div v-if="discardId" class="notice" role="alertdialog" :aria-label='t("确认清理录制残留")'>
        <p>{{ t("永久删除这条录制的源文件和恢复记录？未封装数据也会删除；已导出文件保留。") }}</p>
        <Button :disabled="busy || changingDirectory || recordingActive(view)" @click="discardRecovery(discardId)">{{ t("确认删除源文件") }}</Button>
        <Button variant="outline" :disabled="busy" @click="discardId = null">{{ t("保留") }}</Button>
      </div>
    </div>
    <div class="settings-card">
      <div class="setting-row"><span>{{ t("输出") }}</span><strong>{{ t("MP4 可选声音 / GIF 无声 · 可录制光标") }}</strong></div>
      <div v-if="view" class="setting-row"><span>{{ t("本次音频来源") }}</span><span>{{ t(recordingAudioStatus(view)) }}</span></div>
      <p v-if="view?.systemAudio" class="text-xs text-zinc-500">{{ t("系统无播放或设备静音时，音轨对应区间为静音。") }}</p>
      <div class="setting-row gap-3">
        <span class="min-w-0 flex-1">{{ t("保存位置") }}<small class="break-all">{{ directory?.path ?? (desktop ? t("目录尚未读取") : t("请在桌面应用中选择")) }}</small></span>
        <div class="flex shrink-0 gap-2">
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view)" @click="chooseDirectory()">{{ t("选择文件夹") }}</Button>
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view) || directory?.isDefault" @click="chooseDirectory(true)">{{ t("恢复默认") }}</Button>
        </div>
      </div>
      <div class="setting-row"><span>{{ t("录制时长") }}</span><span>{{ t("不设时长上限") }}</span></div>
      <p v-if="!capabilities?.available" class="py-3 text-sm" role="status">{{ te(capabilities?.reason) || (desktop ? t("正在检查录屏能力…") : t("请在桌面应用中使用录屏。")) }}</p>
      <div v-if="view" class="setting-row"><span>{{ t(recordingStatus(view)) }}</span><span>{{ recordingTime(view.seconds) }} {{ t("· 编码提交") }} {{ view.frames }} {{ t("帧") }}</span></div>
      <div v-if="view" class="setting-row"><span>{{ t("有效帧率") }}</span><span>{{ t(recordingStats(view)) }}</span></div>
      <div v-if="view && view.targetBitrate > 0" class="setting-row"><span>{{ t("目标码率") }}</span><span>{{ (view.targetBitrate / 1_000_000).toFixed(1) }} Mbps</span></div>
      <div v-if="view" class="setting-row"><span>{{ t("编码方式") }}</span><span>{{ t(encoderLabels[view.encoderKind]) }}</span></div>
      <p v-if="view?.encoderFallback" class="notice break-all" role="status">{{ te(view.encoderFallback) }}</p>
      <p v-if="view && view.phase !== 'completed'" class="py-3 text-sm break-all" role="status">{{ te(view.message) }}</p>
      <div class="flex gap-2 py-4">
        <Button v-if="!recordingActive(view)" :disabled="!desktop || busy || changingDirectory || !capabilities?.available" @click="select">{{ t("选择录制范围") }}</Button>
        <Button v-if="view?.phase === 'ready'" :disabled="busy" @click="control('start')">{{ t("开始录制") }}</Button>
        <Button v-if="view?.phase === 'recording'" :disabled="busy" @click="control('pause')">{{ t("暂停") }}</Button>
        <Button v-if="view?.phase === 'paused'" :disabled="busy" @click="control('resume')">{{ t("恢复") }}</Button>
        <Button v-if="view && ['recording', 'paused'].includes(view.phase)" :disabled="busy" @click="control('stop')">{{ t("停止并导出") }}</Button>
        <Button v-if="recordingActive(view)" variant="outline" :disabled="busy" @click="control('cancel')">{{ t("取消并清理") }}</Button>
      </div>
    </div>
  </section>
</template>
