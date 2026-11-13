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
async function copyLogPath() { await api.copyAppDirectory('logs'); message.value = '日志路径已复制。' }
</script>

<template>
  <section class="diagnostics" aria-labelledby="diagnostics-heading">
    <h2 id="diagnostics-heading" class="section-heading">{{ t('记录') }}</h2>
    <div class="settings-card">
      <div class="setting-row">
        <span>{{ t('记录级别') }}<small v-if="status?.level === 'DEBUG'">{{ t('会一直记录调试信息，问题解决后建议改回 INFO') }}</small></span>
        <div class="flex items-center gap-3">
          <span role="status" :class="status && !status.error ? 'status-ok' : 'text-xs text-zinc-500'"><Check v-if="status && !status.error" :size="14" />{{ status?.error ? t("日志不可用") : status ? t('已就绪') : t("状态未读取") }}</span>
          <SelectControl v-if="status" :model-value="status.level" :disabled="!desktop || busy || !status" :aria-label="t('记录级别')" :options="recordingLevels" @update:model-value="changeRecordingLevel" />
        </div>
      </div>
    </div>
    <p v-if="status?.error || error" class="notice error" role="alert">{{ te(error || status?.error) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
    <h2 class="section-heading">{{ t('最近日志') }}<span class="diagnostics-heading-actions"><button type="button" :disabled="!desktop || busy" @click="run(refresh)">{{ busy ? t("处理中…") : t("刷新") }}</button><label class="diagnostics-live">{{ t('自动刷新') }}<input class="mini-switch" :checked="live" :aria-label="t('自动刷新')" type="checkbox" role="switch" :disabled="!desktop || !status || !!status.error" @change="changeLive" /></label></span></h2>
    <div class="diagnostics-viewer">
      <div class="diagnostics-filters">
        <SelectControl v-model="level" :disabled="!desktop || busy" :aria-label="t('显示级别')" :options="[{ value: '', label: t('全部级别') }, ...['DEBUG', 'INFO', 'WARN', 'ERROR'].map(value => ({ value, label: value }))]" @update:model-value="run(() => readLogs())" />
        <SelectControl v-model="module" :disabled="!desktop || busy" :aria-label="t('模块')" :options="[{ value: '', label: t('全部模块') }, ...modules.map(item => ({ value: item.value, label: t(item.label) }))]" @update:model-value="run(() => readLogs())" />
        <input v-model="search" class="diagnostics-search" type="search" :aria-label='t("搜索已读取日志")' :placeholder='t("搜索事件名、错误码")' />
      </div>
      <div class="diagnostics-list" role="region" :aria-label='t("最近日志（最新在前）")' :aria-busy="busy" tabindex="0">
        <p v-if="!visible.length" class="diagnostics-empty">{{ busy ? t("正在读取…") : recent ? t("当前条件下没有日志。") : t("尚未读取日志。") }}</p>
        <div v-for="(entry, index) in visible" :key="index" class="diagnostics-entry" :title="[entry.timestamp, entry.level, entry.module, entry.message].join(' ')">
          <span class="entry-time">{{ entry.timestamp.slice(11, 19) || entry.timestamp }}</span>
          <strong :class="'level-' + entry.level.toLowerCase()">{{ entry.level }}</strong>
          <span class="entry-module">{{ entry.module }}</span><code>{{ entry.message }}</code>
        </div>
      </div>
      <p v-if="recent" class="diagnostics-footer">{{ t("显示") }} {{ visible.length }} / {{ recent.entries.length }} {{ t("条 · 扫描") }} {{ (recent.scannedBytes / 1024).toFixed(0) }} {{ t("KB · 省略") }} {{ recent.omittedLines }} {{ t("条非白名单记录") }}{{ recent.truncated ? t(" · 已达读取或条数上限，非完整历史") : '' }}</p>
    </div>
    <h2 class="section-heading">{{ t('文件') }}</h2>
    <div class="settings-card">
      <div class="setting-row"><span>{{ t('日志目录') }}</span><div class="flex gap-2"><Button variant="outline" :disabled="!desktop || busy" @click="run(() => api.openAppDirectory('logs'))">{{ t("打开") }}</Button><Button variant="outline" :disabled="!desktop || busy || !status?.directory" @click="run(copyLogPath)">{{ t("复制路径") }}</Button></div></div>
      <div class="setting-row"><span>{{ t('诊断包') }}</span><Button variant="outline" :disabled="!desktop || busy || !status?.exportSupported || !!status.error" @click="run(exportLogs)">{{ t("导出…") }}</Button></div>
      <div class="setting-row"><span>{{ t('历史归档') }}</span><Button variant="outline" :disabled="!desktop || busy || !status || !!status.error || !status.archives" @click="run(clearArchives)">{{ t("清理") }}</Button></div>
    </div>
  </section>
</template>

<style scoped>
.diagnostics { display:flex; flex-direction:column; min-height:0; flex:1; }
.diagnostics-heading-actions { display:flex; align-items:center; gap:12px; font-weight:400; }
.diagnostics-heading-actions button { padding:0; border:0; background:transparent; font-size:12px; color:var(--desk-accent-fg,#3b5bdb); cursor:pointer; }
.diagnostics-heading-actions button:disabled { opacity:.45; cursor:default; }
.diagnostics-live { display:flex; align-items:center; gap:6px; font-size:12px; color:var(--desk-muted,#5f6670); }
.diagnostics-viewer { display:flex; flex-direction:column; flex:1; min-height:240px; border:1px solid var(--desk-border,#e3e6ea); border-radius:8px; background:var(--desk-surface,#fff); overflow:hidden; }
.diagnostics-filters { display:flex; align-items:center; gap:8px; padding:8px 10px; border-bottom:1px solid var(--desk-sep,#eceef1); }
.diagnostics-filters :deep(.select-trigger) { min-width:0; font-size:12px; }
.diagnostics-search { flex:1; min-width:120px; font-size:12px !important; }
.diagnostics-list { flex:1; min-height:160px; overflow:auto; font:12px/1.5 ui-monospace,'SF Mono',Consolas,monospace; }
.diagnostics-empty { margin:0; padding:12px; font-family:inherit; color:var(--desk-muted,#5f6670); }
.diagnostics-entry { display:grid; grid-template-columns:58px 46px 84px minmax(0,1fr); gap:8px; padding:4px 12px; border-top:1px solid var(--desk-sep,#eceef1); }
.diagnostics-entry:first-of-type { border-top:0; }
.entry-time, .entry-module { color:var(--desk-muted,#5f6670); }
.diagnostics-entry strong { font-weight:600; }
.level-error { color:var(--desk-danger,#cf222e); }
.level-warn { color:#9a6700; }
.level-debug { color:var(--desk-muted,#5f6670); }
.diagnostics-entry code { font:inherit; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.diagnostics-footer { margin:0; padding:6px 12px; border-top:1px solid var(--desk-sep,#eceef1); font-size:11px; color:var(--desk-muted,#5f6670); }
.diagnostics-list:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:-2px; }
:global([data-theme=dark]) .level-warn { color:#d29922; }
</style>
