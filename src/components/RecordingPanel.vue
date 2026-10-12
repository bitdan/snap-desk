<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import { newerRecording, recordingActive, recordingTime, type RecordingAction, type RecordingView } from '@/lib/recording'

const view = ref<RecordingView | null>(null)
const error = ref('')
const busy = ref(false)
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
onMounted(() => { if (desktop) void poll() })
onUnmounted(() => { disposed = true; clearTimeout(timer) })
</script>

<template>
  <section>
    <h1>录屏</h1>
    <p class="subtitle">框选区域后点击“录屏”；Ctrl / ⌘ + A 选择整个主屏。</p>
    <div v-if="error" class="notice error" role="alert">{{ error }}</div>
    <div class="settings-card">
      <div class="setting-row"><span>输出</span><strong>Windows 系统录屏 · MP4 · 无声 · 10 fps</strong></div>
      <div class="setting-row"><span>单次上限<small>达到任一上限自动结束并导出</small></span><span>3 分钟 / 512 MiB 帧缓存</span></div>
      <div v-if="view" class="setting-row"><span>{{ ({ ready: '待开始', recording: '录制中', paused: '已暂停', encoding: '编码中', completed: '已完成', cancelled: '已取消', failed: '失败' })[view.phase] }}</span><span>{{ recordingTime(view.seconds) }} · {{ view.frames }} 帧</span></div>
      <p v-if="view" class="py-3 text-sm break-all" role="status">{{ view.message }}</p>
      <div class="flex gap-2 py-4">
        <Button v-if="!recordingActive(view)" :disabled="!desktop || busy" @click="select">选择录制范围</Button>
        <Button v-if="view?.phase === 'ready'" :disabled="busy" @click="control('start')">开始录制</Button>
        <Button v-if="view?.phase === 'recording'" :disabled="busy" @click="control('pause')">暂停</Button>
        <Button v-if="view?.phase === 'paused'" :disabled="busy" @click="control('resume')">恢复</Button>
        <Button v-if="view && ['recording', 'paused'].includes(view.phase)" :disabled="busy" @click="control('stop')">停止并导出</Button>
        <Button v-if="recordingActive(view)" variant="outline" :disabled="busy" @click="control('cancel')">取消并清理</Button>
      </div>
    </div>
    <p class="scope-note">确认选区后点击下方“开始录制”才开始计时；控制条显示待开始 / 录制中 / 已暂停 / 正在保存，可暂停、继续、结束保存或取消。MP4 自动保存到应用数据目录的 recordings 文件夹，完成后显示路径。</p>
    <p class="scope-note">使用 Windows 系统视频编码，无需安装外部编码工具。macOS / Linux 原生录屏、窗口录制、声音、光标效果及 GIF / WebP 尚未实现。</p>
  </section>
</template>
