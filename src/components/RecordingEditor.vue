<script setup lang="ts">
import { computed, nextTick, onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { api } from '@/lib/api'
import { recordingClipTime, recordingTrimRange, type RecordingEditorView, type RecordingTrimExport } from '@/lib/recording-editor'

const props = defineProps<{ resultId: number; disabled: boolean }>()
const emit = defineEmits<{ close: [] }>()
const info = ref<RecordingEditorView | null>(null), exported = ref<RecordingTrimExport | null>(null)
const video = ref<HTMLVideoElement | null>(null), closeButton = ref<HTMLButtonElement | null>(null)
const loading = ref(false), busy = ref(false), cancelling = ref(false), ready = ref(false)
const error = ref(''), message = ref(''), previewError = ref('')
const start = ref('0.000'), end = ref('0.000'), currentMs = ref(0)
const outputPreview = ref(false), outputVersion = ref(0)
let disposed = false, selectedPlayback = false, loadingVersion = 0
const previewUrl = computed(() => info.value ? `${convertFileSrc(String(info.value.token), 'recording')}/${outputPreview.value ? 'output' : 'source'}?v=${outputVersion.value}` : '')
const previewAvailable = computed(() => outputPreview.value ? exported.value?.previewAvailable : info.value?.previewAvailable)
const range = computed(() => {
  if (!info.value) return null
  try { return recordingTrimRange(start.value, end.value, info.value.durationMs, info.value.format) }
  catch { return null }
})
const rangeError = computed(() => {
  if (!info.value) return ''
  try { recordingTrimRange(start.value, end.value, info.value.durationMs, info.value.format); return '' }
  catch (e) { return e instanceof Error ? e.message : String(e) }
})
/** 查询在后台读取媒体时长；组件已关闭时立即撤销迟到的原生预览 token。 */
async function load() {
  if (loading.value || busy.value) return
  loading.value = true; error.value = ''; previewError.value = ''; message.value = ''
  const version = ++loadingVersion
  try {
    const next = await api.prepareRecordingEditor(props.resultId)
    if (disposed || version !== loadingVersion) { await api.closeRecordingEditor(next.token); return }
    info.value = next; end.value = (next.durationMs / 1000).toFixed(3)
    start.value = '0.000'; exported.value = null; outputPreview.value = false
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) loading.value = false }
}
function stopPreview() { selectedPlayback = false; video.value?.pause() }
watch(previewUrl, () => { ready.value = false; selectedPlayback = false; previewError.value = ''; currentMs.value = 0 })
function loaded() { ready.value = true }
function timeUpdate() {
  if (!video.value) return
  currentMs.value = Math.round(video.value.currentTime * 1000)
  if (selectedPlayback && range.value && currentMs.value >= range.value.endMs) stopPreview()
}
function setTime(event: Event, which: 'start' | 'end') {
  const value = (event.target as HTMLInputElement).value
  if (which === 'start') start.value = value
  else end.value = value
  stopPreview()
}
function mark(which: 'start' | 'end') {
  if (!info.value || !video.value || outputPreview.value) return
  const ms = Math.max(0, Math.min(info.value.durationMs, Math.round(video.value.currentTime * 1000)))
  if (which === 'start') start.value = (ms / 1000).toFixed(3)
  else end.value = (ms / 1000).toFixed(3)
  stopPreview()
}
async function playSelection() {
  if (!video.value || !ready.value || !range.value || outputPreview.value) return
  video.value.currentTime = range.value.startMs / 1000
  selectedPlayback = true
  try { await video.value.play() }
  catch (e) { selectedPlayback = false; if (!disposed) previewError.value = `无法播放：${String(e)}` }
}
async function save() {
  if (!info.value || !range.value || busy.value || props.disabled || outputPreview.value) return
  const token = info.value.token
  stopPreview(); busy.value = true; error.value = ''; message.value = ''
  try {
    const saved = await api.trimRecording(token, { ...range.value })
    if (disposed || info.value?.token !== token) return
    if (saved) {
      // 移除旧 output 的媒体请求，再允许用户预览新片段。
      outputPreview.value = false; exported.value = saved; outputVersion.value++
      message.value = '片段已另存，原录屏保留。'
    } else message.value = '已取消另存。'
  } catch (e) { if (!disposed && info.value?.token === token) error.value = String(e) }
  finally { if (!disposed) { busy.value = false; cancelling.value = false } }
}
async function cancel() {
  if (!info.value || cancelling.value) return
  cancelling.value = true
  try { await api.cancelRecordingTrim(info.value.token) }
  catch (e) { if (!disposed) { error.value = String(e); cancelling.value = false } }
}
onMounted(() => { void nextTick(() => closeButton.value?.focus()); void load() })
onBeforeUnmount(() => {
  disposed = true; loadingVersion++; stopPreview()
  const media = video.value
  if (media) { media.removeAttribute('src'); media.load() }
  if (info.value) void api.closeRecordingEditor(info.value.token).catch(() => {})
})
</script>

<template>
  <section class="recording-editor" aria-label="录屏预览与裁剪">
    <header><strong>预览与裁剪</strong><button ref="closeButton" @click="emit('close')">关闭预览</button></header>
    <p v-if="loading" role="status">正在读取录屏时长…</p>
    <p v-if="error" class="editor-error" role="alert">{{ error }}</p>
    <button v-if="!info && !loading" @click="load">重新读取</button>
    <template v-if="info">
      <div class="editor-actions"><span>{{ info.format.toUpperCase() }} · 原片 {{ recordingClipTime(info.durationMs) }}</span><button v-if="exported?.previewAvailable" :disabled="busy" @click="stopPreview(); outputPreview = !outputPreview">{{ outputPreview ? '返回原片预览' : '预览裁剪结果' }}</button></div>
      <template v-if="previewAvailable">
        <video v-if="info.format === 'mp4'" ref="video" :src="previewUrl" controls preload="metadata" playsinline :aria-label="outputPreview ? '裁剪结果预览' : '原录屏预览'" @loadedmetadata="loaded" @timeupdate="timeUpdate" @error="ready = false; previewError = '无法在应用内播放此视频，可关闭预览后使用系统播放器。'" />
        <img v-else :src="previewUrl" :alt="outputPreview ? '裁剪后的 GIF 动画' : '原 GIF 动画'" @error="previewError = 'GIF 预览加载失败，可使用系统播放器；原文件仍可另存裁剪。'" />
      </template>
      <p v-else>GIF 超过应用内预览大小限制，仍可选择时间并另存片段。</p>
      <p v-if="previewError" class="editor-error" role="alert">{{ previewError }}</p>
      <p v-if="info.format === 'gif'" class="editor-note">GIF 播放整段动画，裁剪时间精确到 0.01 秒。</p>
      <div class="editor-range">
        <label>开始（秒）<input :value="start" type="number" min="0" :max="info.durationMs / 1000" :step="info.format === 'gif' ? 0.01 : 0.001" :disabled="busy || outputPreview" @input="setTime($event, 'start')" /></label>
        <label>结束（秒）<input :value="end" type="number" min="0" :max="info.durationMs / 1000" :step="info.format === 'gif' ? 0.01 : 0.001" :disabled="busy || outputPreview" @input="setTime($event, 'end')" /></label>
      </div>
      <p v-if="rangeError" class="editor-error" role="alert">{{ rangeError }}</p>
      <p v-else-if="range">所选片段 {{ recordingClipTime(range.endMs - range.startMs) }}</p>
      <div v-if="info.format === 'mp4'" class="editor-actions">
        <span>当前 {{ recordingClipTime(currentMs) }}</span><button :disabled="!ready || outputPreview || busy" @click="mark('start')">当前设为开始</button><button :disabled="!ready || outputPreview || busy" @click="mark('end')">当前设为结束</button><button :disabled="!ready || outputPreview || !range || busy" @click="playSelection">播放所选片段</button>
      </div>
      <div class="editor-actions"><button :disabled="!range || busy || disabled || outputPreview" @click="save">{{ busy ? '正在裁剪并另存…' : '另存所选片段' }}</button><button v-if="busy" :disabled="cancelling" @click="cancel">{{ cancelling ? '正在取消…' : '取消裁剪' }}</button></div>
      <p v-if="disabled" role="status">请先结束当前录制，再另存裁剪片段。</p>
      <p v-if="message" class="editor-success" role="status">{{ message }}</p>
      <p v-if="exported" class="export-path">{{ exported.path }}</p>
    </template>
  </section>
</template>

<style scoped>
.recording-editor { padding: 12px 0; border-top: 1px solid #e1e4e8; font-size: 13px; }
header, .editor-actions, .editor-range { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 8px 0; }
header { justify-content: space-between; }
button { min-height: 32px; padding: 4px 10px; border: 1px solid #d8dee4; border-radius: 4px; }
button:disabled { opacity: .5; }
video, img { display: block; max-width: 100%; width: 100%; max-height: 380px; object-fit: contain; background: #15141b; margin: 8px 0; }
.editor-range label { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
input { width: 120px; min-height: 32px; padding: 4px 8px; border: 1px solid #d8dee4; border-radius: 4px; }
button:focus-visible, input:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.editor-error { color: #b91c1c; overflow-wrap: anywhere; }
.editor-success { color: #24834b; }
.editor-note { color: #57606a; font-size: 12px; }
.export-path { overflow-wrap: anywhere; user-select: text; }
</style>
