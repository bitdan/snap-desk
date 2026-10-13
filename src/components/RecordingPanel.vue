<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import { encoderLabels, recordingStats, newerRecording, recordingActive, recordingStatus, recordingTime, type RecorderCapabilities, type RecordingAction, type RecordingDirectory, type RecordingView } from '@/lib/recording'

const view = ref<RecordingView | null>(null)
const error = ref('')
const busy = ref(false)
const capabilities = ref<RecorderCapabilities | null>(null)
const directory = ref<RecordingDirectory | null>(null)
const changingDirectory = ref(false)
let disposed = false
let timer: ReturnType<typeof setTimeout> | undefined
async function refresh() {
  try { const next = await api.currentRecording(); if (!disposed) view.value = newerRecording(view.value, next) }
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
  try { await api.start() } catch (e) { error.value = String(e) }
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
    <p class="subtitle">框选区域后点击“录屏”；Ctrl / ⌘ + A 选择整个主屏。</p>
    <div v-if="error" class="notice error" role="alert">{{ error }}</div>
    <div class="settings-card">
      <div class="setting-row"><span>输出</span><strong>MP4 · 无声 · 30 / 60 FPS · 可录制光标</strong></div>
      <div class="setting-row gap-3">
        <span class="min-w-0 flex-1">保存位置<small class="break-all">{{ directory?.path ?? (desktop ? '目录尚未读取' : '请在桌面应用中选择') }}</small></span>
        <div class="flex shrink-0 gap-2">
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view)" @click="chooseDirectory()">选择文件夹</Button>
          <Button variant="outline" :disabled="!desktop || busy || changingDirectory || recordingActive(view) || directory?.isDefault" @click="chooseDirectory(true)">恢复默认</Button>
        </div>
      </div>
      <div class="setting-row"><span>单次上限<small>达到上限自动结束并保存</small></span><span>3 分钟</span></div>
      <p v-if="!capabilities?.available" class="py-3 text-sm" role="status">{{ capabilities?.reason ?? (desktop ? '正在检查录屏能力…' : '请在桌面应用中使用录屏。') }}</p>
      <div v-if="view" class="setting-row"><span>{{ recordingStatus(view) }}</span><span>{{ recordingTime(view.seconds) }} · {{ view.frames }} 帧 · 丢弃 {{ view.droppedFrames }} 帧</span></div>
      <div v-if="view" class="setting-row" title="全程平均，不含暂停；编码提交耗时包含色彩转换和同步写入"><span>录屏统计</span><span>{{ recordingStats(view) }}</span></div>
      <div v-if="view" class="setting-row"><span>目标码率</span><span>{{ (view.targetBitrate / 1_000_000).toFixed(1) }} Mbps</span></div>
      <div v-if="view" class="setting-row"><span>编码方式</span><span>{{ encoderLabels[view.encoderKind] }}</span></div>
      <p v-if="view?.encoderFallback" class="notice break-all" role="status">{{ view.encoderFallback }}</p>
      <p v-if="view" class="py-3 text-sm break-all" role="status">{{ view.message }}</p>
      <div class="flex gap-2 py-4">
        <Button v-if="!recordingActive(view)" :disabled="!desktop || busy || changingDirectory || !capabilities?.available" @click="select">选择录制范围</Button>
        <Button v-if="view?.phase === 'ready'" :disabled="busy" @click="control('start')">开始录制</Button>
        <Button v-if="view?.phase === 'recording'" :disabled="busy" @click="control('pause')">暂停</Button>
        <Button v-if="view?.phase === 'paused'" :disabled="busy" @click="control('resume')">恢复</Button>
        <Button v-if="view && ['recording', 'paused'].includes(view.phase)" :disabled="busy" @click="control('stop')">停止并导出</Button>
        <Button v-if="recordingActive(view)" variant="outline" :disabled="busy" @click="control('cancel')">取消并清理</Button>
      </div>
    </div>
    <p class="scope-note">确认选区时可选择自动或软件编码；自动试写失败会尝试软件编码。像素转换仍使用 CPU，高分辨率下可降低帧率或缩小选区。确认选区时可选择立即、3 秒或 5 秒开始；点击控制条“开始录制”后进入倒计时，期间可取消，不计入视频时长。暂停后继续不重复倒计时。MP4 自动保存到上方目录，完成后显示文件路径；目录跨重启记住，恢复默认不会移动已有视频。</p>
    <p class="scope-note">需要 Windows 10 2004 或更新版本，无需安装外部编码工具。帧率为采集目标，静态画面延长上一帧显示，实际帧数取决于采集和编码速度。macOS / Linux、窗口录制、声音、点击效果及 GIF / WebP 尚未实现。</p>
  </section>
</template>
