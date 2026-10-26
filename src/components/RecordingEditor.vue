<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, nextTick, onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Play, Pause, SkipBack, SkipForward, Repeat2, Volume2, VolumeX, Undo2, Redo2, Save, X, RotateCcw, Move, Scissors, Check } from 'lucide-vue-next'
import { api } from '@/lib/api'
import { recordingClipTime, recordingTrimRange, moveRecordingTrim, recordingFrameTime, commitRecordingTrim, travelRecordingTrim, type RecordingEditorView, type RecordingTrimExport, type RecordingTrimHistory, type RecordingTrimRange } from '@/lib/recording-editor'
import type { RecordingReplayLayout, RecordingReplayRegion } from '@/lib/recording'

const props = withDefaults(defineProps<{ resultId: number; disabled: boolean; floating?: boolean; layout?: RecordingReplayLayout | null; frameRate?: number }>(), { floating: false, frameRate: 30 })
const emit = defineEmits<{ close: []; rerecord: [] }>()
const info = ref<RecordingEditorView | null>(null), exported = ref<RecordingTrimExport | null>(null)
const video = ref<HTMLVideoElement | null>(null), stage = ref<HTMLElement | null>(null)
const loading = ref(false), busy = ref(false), cancelling = ref(false), ready = ref(false), playing = ref(false)
const error = ref(''), message = ref(''), previewError = ref('')
const start = ref('0.000'), end = ref('0.000'), currentMs = ref(0)
const speed = ref(1), muted = ref(false), loop = ref(false)
const outputPreview = ref(false), outputVersion = ref(0)
const history = ref<RecordingTrimHistory>({ past: [], present: { startMs: 0, endMs: 0 }, future: [] })
let disposed = false, selectedPlayback = false, loadingVersion = 0
const previewUrl = computed(() => info.value ? `${convertFileSrc(String(info.value.token), 'recording')}/${outputPreview.value ? 'output' : 'source'}?v=${outputVersion.value}` : '')
const previewAvailable = computed(() => outputPreview.value ? exported.value?.previewAvailable : info.value?.previewAvailable)
const duration = computed(() => outputPreview.value ? exported.value?.durationMs ?? 0 : info.value?.durationMs ?? 0)
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
const playbackRange = computed(() => outputPreview.value ? { startMs: 0, endMs: duration.value } : range.value)
const canPlay = computed(() => info.value?.format === 'mp4' && ready.value && !!playbackRange.value && !busy.value && !props.disabled)
const editLocked = computed(() => busy.value || props.disabled || outputPreview.value || !info.value)
const regionStyle = (region?: RecordingReplayRegion) => region ? { left: `${region.left}%`, top: `${region.top}%`, width: `${region.width}%`, height: `${region.height}%` } : undefined
const videoStyle = computed(() => props.floating ? regionStyle(props.layout?.video) : undefined)
const toolsStyle = computed(() => props.floating ? regionStyle(props.layout?.toolbar) : undefined)
const selectedStyle = computed(() => {
  const clip = playbackRange.value
  return clip && duration.value ? { left: `${clip.startMs / duration.value * 100}%`, width: `${(clip.endMs - clip.startMs) / duration.value * 100}%` } : {}
})
/** 元信息、媒体源和事件均属于当前组件；迟到的读取结果只负责关闭自己的 token。 */
async function load() {
  if (loading.value || busy.value) return
  loading.value = true; error.value = ''; previewError.value = ''; message.value = ''
  const version = ++loadingVersion
  try {
    stopPreview()
    if (info.value) { await api.closeRecordingEditor(info.value.token); info.value = null }
    const next = await api.prepareRecordingEditor(props.resultId)
    if (disposed || version !== loadingVersion) { await api.closeRecordingEditor(next.token); return }
    info.value = next; setRange({ startMs: 0, endMs: next.durationMs })
    history.value = { past: [], present: { startMs: 0, endMs: next.durationMs }, future: [] }
    exported.value = null; outputPreview.value = false
    await nextTick(); if (!disposed) stage.value?.focus()
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) loading.value = false }
}
function stopPreview() { selectedPlayback = false; video.value?.pause(); playing.value = false }
watch(previewUrl, () => { stopPreview(); ready.value = false; previewError.value = ''; currentMs.value = 0 })
watch(speed, value => { if (video.value) video.value.playbackRate = value })
watch(() => props.disabled, value => { if (value) stopPreview() })
function loaded() {
  ready.value = true
  if (video.value) { video.value.playbackRate = speed.value; video.value.muted = muted.value }
}
function timeUpdate() {
  const media = video.value, clip = playbackRange.value
  if (!media) return
  currentMs.value = Math.round(media.currentTime * 1000)
  if (selectedPlayback && clip && currentMs.value >= clip.endMs) {
    if (loop.value) {
      // 自定义片段末尾回到起点；整片 ended 事件负责重新播放，不提前清掉循环标志。
      if (!media.ended) media.currentTime = clip.startMs / 1000
    } else stopPreview()
  }
}
async function ended() {
  playing.value = false
  if (loop.value && selectedPlayback && playbackRange.value && video.value) {
    video.value.currentTime = playbackRange.value.startMs / 1000
    try { await video.value.play() } catch { selectedPlayback = false }
  } else selectedPlayback = false
}
function setRange(value: RecordingTrimRange) { start.value = (value.startMs / 1000).toFixed(3); end.value = (value.endMs / 1000).toFixed(3) }
function commitRange() { if (range.value) history.value = commitRecordingTrim(history.value, range.value) }
function setTime(event: Event, which: 'start' | 'end') {
  if (editLocked.value) return
  const value = (event.target as HTMLInputElement).value
  if (which === 'start') start.value = value
  else end.value = value
  stopPreview()
}
/** 拖动时即时预览范围，释放/键盘提交时仅保存一次历史，避免撤销要退回每个像素。 */
function dragEndpoint(event: Event, edge: 'start' | 'end') {
  if (!info.value || editLocked.value) return
  const value = moveRecordingTrim(range.value ?? history.value.present, edge, Number((event.target as HTMLInputElement).value), info.value.durationMs, info.value.format)
  stopPreview(); setRange(value)
  if (video.value && ready.value) { video.value.currentTime = (edge === 'start' ? value.startMs : value.endMs) / 1000; timeUpdate() }
}
function mark(edge: 'start' | 'end') {
  if (!info.value || !video.value || editLocked.value) return
  const value = moveRecordingTrim(range.value ?? history.value.present, edge, currentMs.value, info.value.durationMs, info.value.format)
  stopPreview(); setRange(value); commitRange()
}
function travel(redo = false) {
  if (editLocked.value) return
  stopPreview(); history.value = travelRecordingTrim(history.value, redo); setRange(history.value.present)
}
function seek(event: Event) {
  if (!video.value || !canPlay.value) return
  stopPreview(); video.value.currentTime = Number((event.target as HTMLInputElement).value) / 1000; timeUpdate()
}
function step(direction: -1 | 1) {
  if (!video.value || !canPlay.value || !playbackRange.value) return
  stopPreview(); video.value.currentTime = recordingFrameTime(currentMs.value, direction, props.frameRate, playbackRange.value) / 1000; timeUpdate()
}
async function togglePlayback() {
  const media = video.value, clip = playbackRange.value
  if (!media || !canPlay.value || !clip) return
  if (playing.value) { stopPreview(); return }
  if (currentMs.value < clip.startMs || currentMs.value >= clip.endMs) media.currentTime = clip.startMs / 1000
  selectedPlayback = true
  try { await media.play() }
  catch (e) { selectedPlayback = false; if (!disposed) previewError.value = `无法播放：${String(e)}` }
}
function changeMute() { muted.value = !muted.value; if (video.value) video.value.muted = muted.value }
function close() { if (!busy.value) emit('close') }
function key(event: KeyboardEvent) {
  if (event.isComposing || busy.value) return
  if (event.key === 'Escape') { event.preventDefault(); close(); return }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
    event.preventDefault(); if (!event.repeat) void save(); return
  }
  if ((event.target as HTMLElement).closest('input, select, button')) return
  const command = event.ctrlKey || event.metaKey
  if (command && event.key.toLowerCase() === 'z') { event.preventDefault(); travel(event.shiftKey) }
  else if (!command && event.key === ' ') { event.preventDefault(); if (!event.repeat) void togglePlayback() }
  else if (!command && ['ArrowLeft', 'ArrowRight'].includes(event.key)) { event.preventDefault(); step(event.key === 'ArrowLeft' ? -1 : 1) }
  else if (!command && ['[', ']'].includes(event.key)) { event.preventDefault(); mark(event.key === '[' ? 'start' : 'end') }
}
async function moveWindow(event: PointerEvent) {
  if (!props.floating || busy.value || event.button !== 0) return
  try { await getCurrentWindow().startDragging() } catch (e) { error.value = String(e) }
}
async function save() {
  if (!info.value || !range.value || editLocked.value) return
  const token = info.value.token
  stopPreview(); busy.value = true; error.value = ''; message.value = ''
  try {
    const saved = await api.trimRecording(token, { ...range.value })
    if (disposed || info.value?.token !== token) return
    if (saved) { outputPreview.value = false; exported.value = saved; outputVersion.value++; message.value = '片段已另存，原录屏保留。' }
    else message.value = '已取消另存。'
  } catch (e) { if (!disposed && info.value?.token === token) error.value = String(e) }
  finally { if (!disposed) { busy.value = false; cancelling.value = false } }
}
async function cancel() {
  if (!info.value || cancelling.value) return
  cancelling.value = true
  try { await api.cancelRecordingTrim(info.value.token) }
  catch (e) { if (!disposed) { error.value = String(e); cancelling.value = false } }
}
onMounted(() => { void load() })
onBeforeUnmount(() => {
  disposed = true; loadingVersion++; stopPreview()
  const media = video.value
  if (media) { media.removeAttribute('src'); media.load() }
  if (info.value) void api.closeRecordingEditor(info.value.token).catch(() => {})
})
</script>

<template>
  <section class="recording-editor" :class="{ floating }" :aria-label='t("录屏预览与裁剪")' @keydown="key">
    <div ref="stage" class="editor-stage" :style="videoStyle" tabindex="0" :aria-label='t("录屏画面，空格播放，方向键逐帧定位")'>
      <video v-if="info?.format === 'mp4' && previewAvailable" ref="video" :src="previewUrl" preload="metadata" playsinline @loadedmetadata="loaded" @timeupdate="timeUpdate" @play="playing = true" @pause="playing = false" @ended="ended" @click="togglePlayback" @error="previewError = '应用内无法解码此视频，可关闭预览后使用系统播放器。'" />
      <img v-else-if="info?.format === 'gif' && previewAvailable" :src="previewUrl" :alt="outputPreview ? t('裁剪后的 GIF 动画') : t('原 GIF 动画')" @error="previewError = 'GIF 预览加载失败，可使用系统播放器；原文件仍可另存裁剪。'" />
      <span v-else class="editor-stage-message">{{ loading ? t('正在读取录屏时长…') : info ? t('GIF 超过应用内预览大小限制，仍可选择时间并另存片段。') : t('录屏预览与裁剪') }}</span>
    </div>
    <div class="editor-tools" :style="toolsStyle">
      <div class="editor-timeline">
        <div class="timeline-base" /><div class="timeline-selection" :style="selectedStyle" />
        <input class="timeline-seek" type="range" min="0" :max="duration" step="1" :value="currentMs" :disabled="!canPlay" :aria-label='t("播放进度")' @input="seek" />
        <template v-if="info && !outputPreview">
          <input class="timeline-trim trim-start" type="range" min="0" :max="info.durationMs" :step="info.format === 'gif' ? 10 : 1" :value="range?.startMs ?? 0" :disabled="editLocked" :aria-label='t("裁剪开始端点")' @input="dragEndpoint($event, 'start')" @change="commitRange" />
          <input class="timeline-trim trim-end" type="range" min="0" :max="info.durationMs" :step="info.format === 'gif' ? 10 : 1" :value="range?.endMs ?? info.durationMs" :disabled="editLocked" :aria-label='t("裁剪结束端点")' @input="dragEndpoint($event, 'end')" @change="commitRange" />
        </template>
      </div>
      <div class="editor-transport">
        <button :disabled="!canPlay" :title="playing ? t('暂停预览') : t('播放所选片段')" :aria-label="playing ? t('暂停预览') : t('播放所选片段')" @click="togglePlayback"><Pause v-if="playing" :size="16" /><Play v-else :size="16" /></button>
        <span class="editor-clock">{{ recordingClipTime(currentMs) }} / {{ recordingClipTime(duration) }}</span>
        <button :disabled="!canPlay" :title='t("上一帧（按录制帧率定位）")' :aria-label='t("上一帧（按录制帧率定位）")' @click="step(-1)"><SkipBack :size="15" /></button>
        <button :disabled="!canPlay" :title='t("下一帧（按录制帧率定位）")' :aria-label='t("下一帧（按录制帧率定位）")' @click="step(1)"><SkipForward :size="15" /></button>
        <select v-model.number="speed" :disabled="!canPlay" :aria-label='t("预览播放速度")' :title='t("预览播放速度")'><option v-for="rate in [0.5, 0.75, 1, 1.25, 1.5, 2]" :key="rate" :value="rate">{{ rate }}×</option></select>
        <button :disabled="!canPlay" :aria-pressed="loop" :title='t("循环所选片段")' :aria-label='t("循环所选片段")' @click="loop = !loop"><Repeat2 :size="16" /></button>
        <button :disabled="!canPlay" :aria-pressed="muted" :title='t("静音预览（不改变导出声音）")' :aria-label='t("静音预览（不改变导出声音）")' @click="changeMute"><VolumeX v-if="muted" :size="16" /><Volume2 v-else :size="16" /></button>
        <span class="transport-space" />
        <button :disabled="editLocked || !history.past.length" :title='t("撤销裁剪")' :aria-label='t("撤销裁剪")' @click="travel()"><Undo2 :size="16" /></button>
        <button :disabled="editLocked || !history.future.length" :title='t("重做裁剪")' :aria-label='t("重做裁剪")' @click="travel(true)"><Redo2 :size="16" /></button>
        <button v-if="floating" :disabled="busy || disabled" :title='t("重新录制（保留原片）")' :aria-label='t("重新录制（保留原片）")' @click="emit('rerecord')"><RotateCcw :size="16" /></button>
        <button v-if="floating" :disabled="busy" :title='t("移动回放选区")' :aria-label='t("移动回放选区")' @pointerdown="moveWindow"><Move :size="16" /></button>
        <button class="editor-save" :disabled="editLocked || !range" :title='t("另存所选片段")' :aria-label='t("另存所选片段")' @click="save"><Save :size="16" /></button>
        <button :disabled="busy" :title='t("关闭预览，保留已保存录屏")' :aria-label='t("关闭预览，保留已保存录屏")' @click="close"><X :size="16" /></button>
      </div>
      <div class="editor-range">
        <Scissors :size="14" aria-hidden="true" />
        <label>{{ t('开始（秒）') }}<input :value="start" type="number" min="0" :max="(info?.durationMs ?? 0) / 1000" :step="info?.format === 'gif' ? 0.01 : 0.001" :disabled="editLocked" @input="setTime($event, 'start')" @change="commitRange" /></label>
        <span>—</span>
        <label>{{ t('结束（秒）') }}<input :value="end" type="number" min="0" :max="(info?.durationMs ?? 0) / 1000" :step="info?.format === 'gif' ? 0.01 : 0.001" :disabled="editLocked" @input="setTime($event, 'end')" @change="commitRange" /></label>
        <span v-if="range" class="clip-duration">{{ t('所选片段') }} {{ recordingClipTime(range.endMs - range.startMs) }}</span>
        <span class="transport-space" /><span class="format-tag">{{ info?.format.toUpperCase() }}</span>
        <button v-if="exported?.previewAvailable" :disabled="busy" @click="stopPreview(); outputPreview = !outputPreview">{{ outputPreview ? t('返回原片预览') : t('预览裁剪结果') }}</button>
      </div>
      <div class="editor-feedback" :class="{ 'editor-error': error || previewError || rangeError, 'editor-success': message && !error && !previewError && !rangeError }" :title="te(error || previewError || rangeError) || (exported?.path ?? '')" :role="error || previewError || rangeError ? 'alert' : 'status'">
        <template v-if="busy">{{ t('正在裁剪并另存…') }} <button :disabled="cancelling" @click="cancel">{{ cancelling ? t('正在取消…') : t('取消裁剪') }}</button></template>
        <template v-else-if="error || previewError || rangeError">{{ te(error || previewError || rangeError) }}</template>
        <template v-else-if="message"><Check v-if="message === '片段已另存，原录屏保留。'" :size="12" class="feedback-check" aria-hidden="true" />{{ t(message) }}</template>
        <template v-else-if="info?.format === 'gif'">{{ t('GIF 播放整段动画，裁剪时间精确到 0.01 秒。') }}</template>
        <template v-else>{{ t('拖动蓝色端点保留片段 · 空格播放 · 方向键逐帧 · [ / ] 标记起止') }}</template>
        <button v-if="!info && !loading" @click="load">{{ t('重新读取') }}</button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.recording-editor { font-size:13px; color:#24292f; }
.recording-editor.floating { position:relative; height:100vh; }
.editor-stage { position:relative; height:min(380px, 45vh); background:#15141b; border:2px solid #3b82f6; overflow:hidden; outline:none; }
.editor-stage:focus-visible { border-color:#2563eb; box-shadow:inset 0 0 0 1px #93c5fd; }
.editor-stage video, .editor-stage img { width:100%; height:100%; display:block; object-fit:contain; }
.editor-stage-message { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; color:#e4e4e7; padding:16px; }
.editor-tools { padding:6px; background:#fff; border:1px solid #d8dee4; border-radius:6px; display:grid; grid-template-rows:24px 32px 24px minmax(16px, 1fr); gap:4px; box-shadow:0 2px 8px #0002; }
.floating .editor-stage { position:absolute; left:0; top:0; width:100%; height:calc(100% - 136px); }
.floating .editor-tools { position:absolute; left:0; bottom:0; width:100%; height:128px; }
.editor-timeline { position:relative; margin:0 12px; min-width:0; }
.timeline-base, .timeline-selection { position:absolute; top:9px; height:6px; border-radius:3px; background:#d8dee4; }
.timeline-base { left:0; right:0; }
.timeline-selection { background:#93c5fd; }
.editor-timeline input { appearance:none; -webkit-appearance:none; position:absolute; inset:0; width:100%; height:24px; margin:0; background:transparent; border:0; padding:0; }
.editor-timeline input::-webkit-slider-runnable-track { background:transparent; height:24px; }
.editor-timeline input::-webkit-slider-thumb { appearance:none; -webkit-appearance:none; width:4px; height:24px; border-radius:2px; background:#ef4444; cursor:ew-resize; }
.editor-timeline .timeline-trim { pointer-events:none; }
.editor-timeline .timeline-trim::-webkit-slider-thumb { pointer-events:auto; width:24px; height:24px; border:2px solid #2563eb; background:#eff6ff; border-radius:4px; }
.editor-timeline .trim-start::-webkit-slider-thumb { border-right-width:6px; }
.editor-timeline .trim-end::-webkit-slider-thumb { border-left-width:6px; }
.editor-timeline input:disabled::-webkit-slider-thumb { pointer-events:none; opacity:.45; }
.editor-transport, .editor-range { display:flex; align-items:center; gap:4px; min-width:0; overflow-x:auto; scrollbar-width:none; }
.editor-clock { font-size:12px; font-variant-numeric:tabular-nums; white-space:nowrap; margin:0 4px; }
button, select { font:inherit; color:inherit; background:#fff; border:1px solid transparent; border-radius:4px; min-height:24px; cursor:pointer; }
.editor-transport button { display:flex; align-items:center; justify-content:center; width:28px; min-width:28px; height:28px; padding:0; }
button:hover { background:#f1f5f9; }
button[aria-pressed=true] { color:#2563eb; background:#eff6ff; }
button:disabled, select:disabled { opacity:.4; cursor:default; }
button:focus-visible, input:focus-visible, select:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.editor-save { color:#2563eb; background:#eff6ff; }
.transport-space { flex:1; min-width:4px; }
.editor-range label { display:flex; align-items:center; gap:4px; white-space:nowrap; font-size:12px; }
.editor-range input { width:82px; height:24px; border:1px solid #d8dee4; border-radius:3px; padding:2px 4px; font:inherit; font-variant-numeric:tabular-nums; }
.clip-duration, .format-tag { font-size:12px; white-space:nowrap; color:#57606a; }
.format-tag { padding:2px 6px; border:1px solid #e1e4e8; border-radius:3px; }
.editor-range button { font-size:12px; white-space:nowrap; border-color:#d8dee4; padding:0 6px; }
.editor-feedback { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:#71717a; font-size:12px; line-height:16px; }
.editor-feedback button { min-height:16px; height:16px; font-size:12px; padding:0 4px; color:#2563eb; }
.feedback-check { display:inline-block; vertical-align:middle; margin-right:4px; }
.editor-error { color:#b91c1c; }.editor-success { color:#24834b; }
</style>
