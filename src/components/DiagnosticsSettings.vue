<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from './ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import { debugSeconds, searchLogs, type DiagnosticsStatus, type LogLevel, type LogModule, type RecentLogs } from '@/lib/diagnostics'

const status = ref<DiagnosticsStatus | null>(null)
const recent = ref<RecentLogs | null>(null)
const level = ref<LogLevel | ''>(''), module = ref<LogModule | ''>(''), search = ref('')
const busy = ref(false), error = ref(''), message = ref('')
const loadedAt = ref(0), now = ref(performance.now())
const remaining = computed(() => debugSeconds(status.value?.debugRemainingSeconds ?? 0, loadedAt.value, now.value))
const visible = computed(() => searchLogs(recent.value?.entries ?? [], search.value))
const modules: { value: LogModule; label: string }[] = [
  { value: 'app', label: '应用' }, { value: 'capture', label: '截图' }, { value: 'scroll', label: '长截图' },
  { value: 'recording', label: '录屏' }, { value: 'ocr', label: 'OCR' }, { value: 'translation', label: '翻译' },
  { value: 'clipboard', label: '剪贴板' }, { value: 'frontend', label: '前端' }, { value: 'diagnostics', label: '诊断' },
]
function acceptStatus(value: DiagnosticsStatus) {
  status.value = value
  loadedAt.value = now.value = performance.now()
}
async function readLogs() {
  // 刷新失败时清掉旧列表，防止筛选标签与旧数据不一致。
  recent.value = null
  recent.value = await api.diagnosticsRecentLogs({ limit: 500, level: level.value || null, module: module.value || null })
}
async function run(action: () => Promise<void>) {
  if (!desktop || busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await action() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function refresh() {
  status.value = null
  recent.value = null
  const value = await api.diagnosticsStatus()
  acceptStatus(value)
  if (!value.error) await readLogs()
}
async function toggleDebug() {
  acceptStatus(await api.diagnosticsSetDebug(remaining.value === 0))
  message.value = remaining.value > 0 ? 'DEBUG 已立即生效，30 分钟后自动关闭；重启也会恢复 INFO。' : '已恢复 INFO。'
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
  // 仅本地倒计时，不每秒调用 IPC 或读取文件。
  timer = setInterval(() => { now.value = performance.now() }, 1000)
})
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <section class="diagnostics" aria-labelledby="diagnostics-heading">
    <div class="diagnostics-toolbar">
      <h2 id="diagnostics-heading">运行状态</h2>
      <span role="status" :class="status && !status.error ? 'status-ok' : 'text-xs text-zinc-500'"><Check v-if="status && !status.error" :size="14" />{{ status?.error ? '日志不可用' : status ? (remaining > 0 ? `DEBUG · 剩余 ${Math.ceil(remaining / 60)} 分钟` : 'INFO') : '状态未读取' }}</span>
      <Button variant="ghost" :disabled="!desktop || busy" @click="run(refresh)">{{ busy ? '处理中…' : '刷新' }}</Button>
    </div>
    <p class="text-xs text-zinc-500">日志仅保存在本机，导出分享前请检查内容。</p>
    <p v-if="status && !status.error" class="mt-2 text-xs text-zinc-500">日志共 {{ (status.totalBytes / 1024 / 1024).toFixed(2) }} MB · {{ status.archives }} 份归档 · 最近 500 条有效记录中 {{ status.recentErrors }} 条错误</p>
    <p v-if="status?.error || error" class="notice error" role="alert">{{ error || status?.error }}</p>
    <p v-if="message" class="notice" role="status">{{ message }}</p>
    <div class="diagnostics-toolbar mt-3">
      <label>级别 <select v-model="level" :disabled="!desktop || busy" @change="run(readLogs)"><option value="">全部</option><option v-for="value in ['ERROR', 'WARN', 'INFO', 'DEBUG']" :key="value">{{ value }}</option></select></label>
      <label>模块 <select v-model="module" :disabled="!desktop || busy" @change="run(readLogs)"><option value="">全部</option><option v-for="item in modules" :key="item.value" :value="item.value">{{ item.label }}</option></select></label>
      <input v-model="search" class="diagnostics-search" type="search" aria-label="搜索已读取日志" placeholder="搜索事件名、错误码" />
    </div>
    <div class="diagnostics-list" role="region" aria-label="最近日志（最新在前）" :aria-busy="busy" tabindex="0">
      <p v-if="!visible.length" class="p-3 text-xs text-zinc-500">{{ busy ? '正在读取…' : recent ? '当前条件下没有日志。' : '尚未读取日志。' }}</p>
      <div v-for="(entry, index) in visible" :key="index" class="diagnostics-entry">
        <span class="text-zinc-500">{{ entry.timestamp }}</span>
        <strong :class="{ 'text-red-700': entry.level === 'ERROR', 'text-amber-700': entry.level === 'WARN' }">{{ entry.level }}</strong>
        <span>{{ entry.module }}</span><code>{{ entry.message }}</code>
      </div>
    </div>
    <p v-if="recent" class="mt-2 text-xs text-zinc-500">显示 {{ visible.length }} / {{ recent.entries.length }} 条 · 扫描 {{ (recent.scannedBytes / 1024).toFixed(0) }} KB · 省略 {{ recent.omittedLines }} 条非白名单记录{{ recent.truncated ? ' · 已达读取或条数上限，非完整历史' : '' }}</p>
    <div class="diagnostics-toolbar mt-3">
      <Button variant="outline" :disabled="!desktop || busy" @click="run(() => api.openAppDirectory('logs'))">打开日志目录</Button>
      <Button variant="ghost" :disabled="!desktop || busy || !status?.directory" @click="run(async () => { await api.copyAppDirectory('logs'); message = '日志路径已复制。' })">复制路径</Button>
      <Button variant="outline" :disabled="!desktop || busy || !status?.exportSupported || !!status.error" @click="run(exportLogs)">导出诊断包</Button>
      <Button variant="outline" :disabled="!desktop || busy || !status || !!status.error || !status.archives" @click="run(clearArchives)">清理历史归档</Button>
      <Button variant="ghost" :disabled="!desktop || busy || !status" :aria-pressed="remaining > 0" @click="run(toggleDebug)">{{ remaining > 0 ? '关闭详细日志' : '临时开启 DEBUG（30 分钟）' }}</Button>
    </div>
    <p v-if="status && !status.exportSupported" class="mt-2 text-xs text-zinc-500">当前平台暂不支持 ZIP 导出；可打开日志目录查看文件。</p>
  </section>
</template>

<style scoped>
.diagnostics { display: flex; flex-direction: column; gap: 8px; min-height: 0; flex: 1; }
.diagnostics h2 { font-size: 14px; font-weight: 600; margin: 0; }
.diagnostics-toolbar { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.diagnostics-toolbar label { display: flex; align-items: center; gap: 4px; font-size: 12px; }
.diagnostics select, .diagnostics input { height: 32px; border: 1px solid #e1e4e8; border-radius: 4px; padding: 4px 8px; background: white; font-size: 13px; }
.diagnostics-search { flex: 1; min-width: 160px; }
.diagnostics-list { flex: 1; min-height: 160px; overflow: auto; border: 1px solid #e1e4e8; border-radius: 4px; }
.diagnostics-entry { display: flex; flex-wrap: wrap; gap: 4px 8px; padding: 8px; border-bottom: 1px solid #eee; font-size: 12px; }
.diagnostics-entry code { flex-basis: 100%; overflow-wrap: anywhere; white-space: pre-wrap; }
.diagnostics :is(select, input, .diagnostics-list):focus-visible { outline: 2px solid #2563eb; outline-offset: 2px; }
</style>
