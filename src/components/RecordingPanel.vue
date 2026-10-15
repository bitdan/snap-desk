<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import type { RecoverableRecording, RecordingResult, RecordingResultAction } from '@/lib/recording'
import { encoderLabels, recordingStats, recordingStageTimes, recordingCopyStats, newerRecording, recordingActive, recordingStatus, recordingTime, type RecorderCapabilities, type RecordingAction, type RecordingDirectory, type RecordingView } from '@/lib/recording'

const view = ref<RecordingView | null>(null)
const error = ref('')
const busy = ref(false)
const capabilities = ref<RecorderCapabilities | null>(null)
const directory = ref<RecordingDirectory | null>(null)
const changingDirectory = ref(false)
const recoveries = ref<RecoverableRecording[]>([])
const recoveredPath = ref('')
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
    if (result.value?.id === id) resultMessage.value = action === 'copyPath' ? '路径已复制。' : action === 'preview' ? '已请求系统播放器打开录屏。' : '已打开保存目录。'
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
let timer: ReturnType<typeof setTimeout> | undefined
async function refresh() {
  try {
    const next = await api.currentRecording()
    if (!disposed) {
      const ended = recordingActive(view.value) && !recordingActive(next)
      view.value = newerRecording(view.value, next)
      await loadResult()
      if (ended) await loadRecoveries()
    }
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
// 串行轮询，不在窗口后台堆积 IPC；录屏状态归 Rust 所有，重开面板仍可恢复。
async function poll() {
  await refresh()
  if (!disposed) timer = setTimeout(poll, 750)
}
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
  void poll()
  void loadRecoveries()
  void api.currentRecordingDirectory().then(value => { if (!disposed) directory.value = value })
    .catch(e => { if (!disposed) error.value = String(e) })
  void api.recordingCapabilities().then(value => { if (!disposed) capabilities.value = value })
    .catch(e => { if (!disposed) error.value = String(e) })
})
onUnmounted(() => { disposed = true; clearTimeout(timer) })
</script>

<template>
  <section>
    <h1>录屏</h1>
    <p class="subtitle">选择录制范围后设置录屏选项。录屏快捷键可在设置中修改，录制中再次按下停止并保存。</p>
    <div v-if="error" class="notice error" role="alert">{{ error }}</div>
    <p v-if="recoveredPath" class="notice break-all" role="status">已重新导出：{{ recoveredPath }}</p>
    <div v-if="result" class="settings-card mb-3">
      <div class="setting-row"><strong>最近完成的录屏</strong><span class="text-sm text-zinc-500">本次运行</span></div>
      <p class="py-2 break-all font-medium">{{ result.filename }}</p>
      <p class="break-all text-xs text-zinc-500">{{ result.path }}</p>
      <div class="flex flex-wrap gap-2 py-4">
        <Button :disabled="resultBusy || recordingActive(view)" title="使用系统默认播放器打开 MP4 / GIF" @click="resultAction('preview')">预览（系统播放器）</Button>
        <Button variant="outline" :disabled="resultBusy" @click="resultAction('openDirectory')">打开目录</Button>
        <Button variant="ghost" :disabled="resultBusy" @click="resultAction('copyPath')">复制路径</Button>
      </div>
      <p v-if="resultMessage" class="text-sm pb-3" role="status">{{ resultMessage }}</p>
    </div>
    <div v-if="recoveries.length" class="settings-card mb-3">
      <div class="setting-row"><strong>待恢复视频</strong><Button variant="outline" :disabled="busy" @click="loadRecoveries">刷新</Button></div>
      <p class="py-2 text-sm">已封装视频可重新导出到当前保存位置；也可先选择其他文件夹。</p>
      <div v-for="item in recoveries" :key="item.id" class="setting-row gap-3">
        <span class="min-w-0 flex-1 break-all">{{ item.path }}<small>{{ item.finalized ? '封装已完成，源文件已保留' : '封装未确认完成，源文件保留供检查' }}</small></span>
        <Button v-if="item.finalized" :disabled="busy || changingDirectory || recordingActive(view)" @click="retryExport(item.id)">重试导出</Button>
      </div>
    </div>
    <div class="settings-card">
      <div class="setting-row"><span>输出</span><strong>MP4 / GIF · 无声 · 可录制光标</strong></div>
      <div class="setting-row gap-3">
        <span class="min-w-0 flex-1">保存位置<small class="break-all">{{ directory?.path ?? (desktop ? '目录尚未读取' : '请在桌面应用中选择') }}</small></span>
        <div class="flex shrink-0 gap-2">
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view)" @click="chooseDirectory()">选择文件夹</Button>
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view) || directory?.isDefault" @click="chooseDirectory(true)">恢复默认</Button>
        </div>
      </div>
      <div class="setting-row"><span>录制时长</span><span>不设时长上限</span></div>
      <p v-if="!capabilities?.available" class="py-3 text-sm" role="status">{{ capabilities?.reason ?? (desktop ? '正在检查录屏能力…' : '请在桌面应用中使用录屏。') }}</p>
      <div v-if="view" class="setting-row"><span>{{ recordingStatus(view) }}</span><span>{{ recordingTime(view.seconds) }} · 编码提交 {{ view.frames }} 帧</span></div>
      <div v-if="view" class="setting-row" title="成功提交编码帧的媒体时间窗口，不含暂停；恢复后重置近两秒窗口。静态画面可自然降至 0 FPS，并非丢帧。"><span>有效帧率</span><span>{{ recordingStats(view) }}</span></div>
      <details v-if="view" class="py-3 text-sm">
        <summary class="cursor-pointer">录屏诊断</summary>
        <div class="setting-row"><span>采集 / 合成 / 入队</span><span>{{ view.stats.capturedFrames }} / {{ view.stats.composedFrames }} / {{ view.stats.queuedFrames }} 帧</span></div>
        <div class="setting-row"><span>帧率节流</span><span>{{ view.stats.throttledSamples }} 次</span></div>
        <div class="setting-row"><span>队列满 / 旧代次丢弃</span><span>{{ view.stats.queueDroppedFrames }} / {{ view.stats.staleQueuedFrames }} 帧</span></div>
        <div class="setting-row"><span>采集 / 合成后失效</span><span>{{ view.stats.rejectedCaptureFrames }} / {{ view.stats.rejectedComposedFrames }} 帧</span></div>
        <p class="py-2">{{ recordingStageTimes(view) }}</p>
        <p class="pb-2">{{ recordingCopyStats(view) }}</p>
        <p class="pb-2 text-zinc-500">NV12 转换直接写入编码缓冲，输出量不代表额外复制；统计不含 GPU 和系统编码器内部传输。</p>
        <p class="text-zinc-500">采集耗时按 {{ view.stats.capturePolls }} 次取样平均，包含空取样、GPU 回读和像素复制，不含等待帧事件。合成仅统计键鼠特效重绘；编码提交包含色彩转换与同步写入，不代表 GPU 内部延迟。</p>
        <p class="pt-2 text-zinc-500">采集统计后端实际取出的帧，不含系统帧池内部丢弃；动画可复用底图生成多帧。节流次数包含延后重绘，不能与各阶段帧数相加。暂停、停止期间的帧会单独记为失效。</p>
      </details>
      <div v-if="view && view.targetBitrate > 0" class="setting-row"><span>目标码率</span><span>{{ (view.targetBitrate / 1_000_000).toFixed(1) }} Mbps</span></div>
      <div v-if="view" class="setting-row"><span>编码方式</span><span>{{ encoderLabels[view.encoderKind] }}</span></div>
      <p v-if="view?.encoderFallback" class="notice break-all" role="status">{{ view.encoderFallback }}</p>
      <p v-if="view && view.phase !== 'completed'" class="py-3 text-sm break-all" role="status">{{ view.message }}</p>
      <div class="flex gap-2 py-4">
        <Button v-if="!recordingActive(view)" :disabled="!desktop || busy || changingDirectory || !capabilities?.available" @click="select">选择录制范围</Button>
        <Button v-if="view?.phase === 'ready'" :disabled="busy" @click="control('start')">开始录制</Button>
        <Button v-if="view?.phase === 'recording'" :disabled="busy" @click="control('pause')">暂停</Button>
        <Button v-if="view?.phase === 'paused'" :disabled="busy" @click="control('resume')">恢复</Button>
        <Button v-if="view && ['recording', 'paused'].includes(view.phase)" :disabled="busy" @click="control('stop')">停止并导出</Button>
        <Button v-if="recordingActive(view)" variant="outline" :disabled="busy" @click="control('cancel')">取消并清理</Button>
      </div>
    </div>
  </section>
</template>
