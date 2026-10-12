<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { Pause, Play, Square, X } from 'lucide-vue-next'
import { api } from '@/lib/api'
import { newerRecording, recordingActive, recordingLabels, recordingTime, type RecordingAction, type RecordingView } from '@/lib/recording'

const view = ref<RecordingView | null>(null)
const busy = ref(false)
const error = ref('')
let disposed = false
let timer: ReturnType<typeof setTimeout> | undefined
// 控制条窗口复用且保持挂载；新会话清除上次错误，顺序号拦截迟到查询。
async function refresh() {
  const next = await api.currentRecording()
  if (disposed) return
  if (next && next.id !== view.value?.id) error.value = ''
  view.value = newerRecording(view.value, next)
}
async function poll() {
  try { await refresh() } catch (e) { if (!disposed) error.value = String(e) }
  if (!disposed) timer = setTimeout(poll, 250)
}
async function control(action: RecordingAction) {
  if (!view.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.controlRecording(view.value.id, action); await refresh() }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
onMounted(poll)
onUnmounted(() => { disposed = true; clearTimeout(timer) })
</script>

<template>
  <main class="recording-controls" aria-label="录屏控制条">
    <span class="dot" :class="{ paused: view?.phase !== 'recording' }" />
    <span class="time" role="timer">{{ recordingTime(view?.seconds ?? 0) }}</span>
    <span class="status" role="status">{{ view ? recordingLabels[view.phase] : '连接中…' }}</span>
    <span class="fps">目标 10 FPS</span>
    <button v-if="view?.phase === 'ready'" class="start" :disabled="busy" @click="control('start')"><Play :size="15" />开始录制</button>
    <button v-else-if="view?.phase === 'paused'" :disabled="busy" @click="control('resume')"><Play :size="15" />继续</button>
    <button v-else :disabled="busy || view?.phase !== 'recording'" @click="control('pause')"><Pause :size="15" />暂停</button>
    <button :disabled="busy || !view || !['recording', 'paused'].includes(view.phase)" @click="control('stop')"><Square :size="15" />结束保存</button>
    <button :disabled="busy || !recordingActive(view)" title="丢弃本次录屏" @click="control('cancel')"><X :size="15" />取消</button>
    <span v-if="error" class="error" role="alert" :title="error">{{ error }}</span>
  </main>
</template>

<style scoped>
.recording-controls { height: 100vh; display: flex; align-items: center; justify-content: center; gap: 8px; padding: 5px 8px; background: #fafafa; border: 1px solid #ddd; color: #27272a; font-size: 12px; overflow: hidden; }
.dot { flex: none; width: 8px; height: 8px; border-radius: 50%; background: #ef4444; }
.dot.paused { background: #a1a1aa; }
.time { font-variant-numeric: tabular-nums; white-space: nowrap; }
.status { min-width: 42px; color: #71717a; white-space: nowrap; }
button { display: flex; align-items: center; justify-content: center; gap: 3px; flex: none; padding: 0 7px; height: 30px; border: 1px solid #ddd; border-radius: 5px; white-space: nowrap; }
.start { background: #7c3aed; color: white; border-color: #7c3aed; }
.fps { color: #71717a; font-size: 11px; white-space: nowrap; }
button:hover { background: #ede9fe; color: #7c3aed; }
button:disabled { opacity: .4; }
.error { position: absolute; bottom: 0; left: 4px; right: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #b91b1b; font-size: 10px; }
</style>
