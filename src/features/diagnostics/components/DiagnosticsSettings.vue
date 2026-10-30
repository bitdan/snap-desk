<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'
import { searchLogs, type DiagnosticsStatus, type LogLevel, type LogModule, type RecentLogs } from '@/features/diagnostics/diagnostics'

const status = ref<DiagnosticsStatus | null>(null)
const recent = ref<RecentLogs | null>(null)
const level = ref<LogLevel | ''>(''), module = ref<LogModule | ''>(''), search = ref('')
const busy = ref(false), error = ref(''), message = ref('')
const live = ref(true)
const recordingLevels = (['DEBUG', 'INFO', 'WARN', 'ERROR'] as const).map(value => ({ value, label: value }))
function changeLive(event: Event) { live.value = (event.target as HTMLInputElement).checked }
let disposed = false
let backgroundRead: Promise<void> | undefined
const visible = computed(() => searchLogs(recent.value?.entries ?? [], search.value))
const modules: { value: LogModule; label: string }[] = [
  { value: 'app', label: '应用' }, { value: 'capture', label: '截图' }, { value: 'scroll', label: '滚动截图' },
  { value: 'recording', label: '录屏' }, { value: 'ocr', label: 'OCR' }, { value: 'translation', label: '翻译' },
  { value: 'clipboard', label: '剪贴板' }, { value: 'frontend', label: '前端' }, { value: 'diagnostics', label: '诊断' },
]
function acceptStatus(value: DiagnosticsStatus) {
  status.value = value
}
async function readLogs(preserve = false) {
  // 切换筛选清掉旧结果；自动刷新保留列表，避免闪烁和打断阅读。
  if (!preserve) recent.value = null
  const query = { limit: 500, level: level.value || null, module: module.value || null }
  const value = await api.diagnosticsRecentLogs(query)
  if (!disposed && query.level === (level.value || null) && query.module === (module.value || null)) recent.value = value
}
/** 后台读取不切换控件状态；用户操作等待正在读取的任务，避免诊断 IPC 锁冲突。 */
async function run(action: () => Promise<void>, background = false) {
  if (!desktop || disposed || busy.value) return
  if (background) {
    if (backgroundRead) return
    const pending = (async () => {
      try { await action(); if (!disposed) error.value = '' }
      catch (e) { if (!disposed) { error.value = String(e); live.value = false } }
    })()
    backgroundRead = pending
    try { await pending }
    finally { if (backgroundRead === pending) backgroundRead = undefined }
    return
  }
  busy.value = true; error.value = ''; message.value = ''
  try {
    await backgroundRead
    if (disposed) return
    await action()
    if (!disposed) error.value = ''
  }
  catch (e) { if (!disposed) { error.value = String(e); live.value = false } }
  finally { busy.value = false }
}
async function refresh() {
  // 已有状态和列表直接更新，重复刷新不让状态回到“未读取”或卸载级别选择器。
  const value = await api.diagnosticsStatus()
  if (disposed) return
  acceptStatus(value)
  if (!value.error) await readLogs(true)
}
async function changeRecordingLevel(value: LogLevel) {
  await run(async () => {
    acceptStatus(await api.diagnosticsSetLevel(value))
    message.value = '日志记录级别已更新。'
  })
}
async function exportLogs() {
  const saved = await api.diagnosticsExport()
  message.value = saved ? '诊断包已保存。分享前请检查内容。' : '已取消导出。'
}
async function clearArchives() {
  const result = await api.diagnosticsClearArchives()
  if (result.cancelled) { message.value = '已取消清理。'; return }
  message.value = `已清理 ${result.deleted} 份归档，${result.failed} 份未能清理；当前日志保留。`
  await refresh()
}
let timer: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  void run(refresh)
  // 仅日志页可见时读取有界尾部；同一时刻只允许一个诊断操作。
  timer = setInterval(() => {
    if (live.value && !busy.value && status.value && !status.value.error && document.visibilityState !== 'hidden') {
      void run(() => readLogs(true), true)
    }
  }, 2000)
})
onUnmounted(() => { disposed = true; if (timer) clearInterval(timer) })
</script>

<template>
  <section class="diagnostics" aria-labelledby="diagnostics-heading">
    <div class="diagnostics-toolbar">
      <label id="diagnostics-heading">{{ t('记录级别') }} <SelectControl v-if="status" :model-value="status.level" :disabled="!desktop || busy || !status" :aria-label="t('记录级别')" :options="recordingLevels" @update:model-value="changeRecordingLevel" /></label>
      <span role="status" :class="status && !status.error ? 'status-ok' : 'text-xs text-zinc-500'"><Check v-if="status && !status.error" :size="14" />{{ status?.error ? t("日志不可用") : status ? t('已就绪') : t("状态未读取") }}</span>
      <Button variant="ghost" :disabled="!desktop || busy" @click="run(refresh)">{{ busy ? t("处理中…") : t("刷新") }}</Button>
      <label class="diagnostics-live"><input :checked="live" :aria-label="t('自动刷新')" @change="changeLive" type="checkbox" :disabled="!desktop || !status || !!status.error" />{{ t('自动刷新') }}</label>
    </div>
    <p v-if="status?.error || error" class="notice error" role="alert">{{ te(error || status?.error) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
    <div class="diagnostics-toolbar mt-3">
      <label>{{ t("显示级别") }} <SelectControl v-model="level" :disabled="!desktop || busy" :aria-label="t('显示级别')" @update:model-value="run(() => readLogs())" :options="[{ value: '', label: 'ALL' }, ...['DEBUG', 'INFO', 'WARN', 'ERROR'].map(value => ({ value, label: value }))]" /></label>
      <label>{{ t("模块") }} <SelectControl v-model="module" :disabled="!desktop || busy" :aria-label="t('模块')" @update:model-value="run(() => readLogs())" :options="[{ value: '', label: t('全部') }, ...modules.map(item => ({ value: item.value, label: t(item.label) }))]" /></label>
      <input v-model="search" class="diagnostics-search" type="search" :aria-label='t("搜索已读取日志")' :placeholder='t("搜索事件名、错误码")' />
    </div>
    <div class="diagnostics-list" role="region" :aria-label='t("最近日志（最新在前）")' :aria-busy="busy" tabindex="0">
      <p v-if="!visible.length" class="p-3 text-xs text-zinc-500">{{ busy ? t("正在读取…") : recent ? t("当前条件下没有日志。") : t("尚未读取日志。") }}</p>
      <div v-for="(entry, index) in visible" :key="index" class="diagnostics-entry">
        <span class="text-zinc-500">{{ entry.timestamp }}</span>
        <strong :class="{ 'text-red-700': entry.level === 'ERROR', 'text-amber-700': entry.level === 'WARN' }">{{ entry.level }}</strong>
        <span>{{ entry.module }}</span><code>{{ entry.message }}</code>
      </div>
    </div>
    <p v-if="recent" class="mt-2 text-xs text-zinc-500">{{ t("显示") }} {{ visible.length }} / {{ recent.entries.length }} {{ t("条 · 扫描") }} {{ (recent.scannedBytes / 1024).toFixed(0) }} {{ t("KB · 省略") }} {{ recent.omittedLines }} {{ t("条非白名单记录") }}{{ recent.truncated ? t(" · 已达读取或条数上限，非完整历史") : '' }}</p>
    <div class="diagnostics-toolbar mt-3">
      <Button variant="outline" :disabled="!desktop || busy" @click="run(() => api.openAppDirectory('logs'))">{{ t("打开日志目录") }}</Button>
      <Button variant="ghost" :disabled="!desktop || busy || !status?.directory" @click="run(async () => { await api.copyAppDirectory('logs'); message = '日志路径已复制。' })">{{ t("复制路径") }}</Button>
      <Button variant="outline" :disabled="!desktop || busy || !status?.exportSupported || !!status.error" @click="run(exportLogs)">{{ t("导出诊断包") }}</Button>
      <Button variant="outline" :disabled="!desktop || busy || !status || !!status.error || !status.archives" @click="run(clearArchives)">{{ t("清理历史归档") }}</Button>

    </div>
    <p class="text-xs text-zinc-500">{{ t('记录级别控制后续写入，显示级别仅筛选已有日志；记录级别会在本机保留。') }}</p>
    <p v-if="status && !status.exportSupported" class="mt-2 text-xs text-zinc-500">{{ t("此平台不支持导出，请打开日志目录。") }}</p>
  </section>
</template>

<style scoped>
.diagnostics { display: flex; flex-direction: column; gap: 8px; min-height: 0; flex: 1; }
.diagnostics h2 { font-size: 14px; font-weight: 600; margin: 0; }
.diagnostics-toolbar { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.diagnostics-toolbar label { display: flex; align-items: center; gap: 4px; font-size: 12px; }
.diagnostics select, .diagnostics input { height: 32px; border: 1px solid #e1e4e8; border-radius: 4px; padding: 4px 8px; background: white; font-size: 13px; }
.diagnostics .diagnostics-live input { width:14px; height:14px; padding:0; accent-color:#2563eb; }
.diagnostics-search { flex: 1; min-width: 160px; }
.diagnostics-list { flex: 1; min-height: 160px; overflow: auto; border: 1px solid #e1e4e8; border-radius: 4px; }
.diagnostics-entry { display: flex; flex-wrap: wrap; gap: 4px 8px; padding: 8px; border-bottom: 1px solid #eee; font-size: 12px; }
.diagnostics-entry code { flex:1; min-width:240px; overflow-wrap: anywhere; white-space: pre-wrap; }
.diagnostics :is(select, input, .diagnostics-list):focus-visible { outline: 2px solid #2563eb; outline-offset: 2px; }
</style>
