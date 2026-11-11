<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { ArrowLeft, Check, Copy, Eraser, Image as ImageIcon, Languages, RotateCcw, WrapText } from 'lucide-vue-next'
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { api } from '@/services/api'
import Button from '@/shared/ui/button/Button.vue'
import TranslationPanel from '@/features/recognition/components/TranslationPanel.vue'
import type { OcrResult } from '@/features/recognition/ai'
import { lineRanges, mergeLines, ocrDisplaySize, ocrImageView, tidySpaces, type OcrImage } from '@/features/recognition/ocr-workspace'

/**
 * 截图 / 贴图共用的识别工作区：左侧原图与逐行识别框，右侧可编辑文字或就地翻译。
 * 草稿由调用方持有，识别与翻译视图来回切换不丢修正。
 */
const props = defineProps<{ result: OcrResult; image?: OcrImage | null; mode?: 'ocr' | 'translate'; error?: string; scale?: number }>()
const text = defineModel<string>('text', { required: true })
const emit = defineEmits<{ close: [] }>()

const view = ref<'ocr' | 'translate'>(props.mode ?? 'ocr')
watch(() => props.mode, value => { if (value) view.value = value })
const imageKey = 'snap-desk.ocr-show-image.v1'
const showImage = ref((() => { try { return localStorage.getItem(imageKey) !== '0' } catch { return true } })())
function toggleImage() {
  showImage.value = !showImage.value
  try { localStorage.setItem(imageKey, showImage.value ? '1' : '0') } catch { /* 偏好写不进去仍允许本次切换。 */ }
}
const busy = ref(false), message = ref(''), localError = ref('')
const editor = ref<HTMLTextAreaElement | null>(null)
const activeLine = ref<string | null>(null)

const lines = computed(() => [...(props.result.lines ?? [])].sort((a, b) => a.order - b.order))
const imageView = computed(() => props.image ? ocrImageView(props.image, props.result, lines.value) : null)
// 原图按截图时的屏幕尺寸显示（物理像素 ÷ 缩放比），空间不足时只等比缩小，不会被拉伸放大。
const displaySize = computed(() => props.image ? ocrDisplaySize(props.image, props.scale ?? globalThis.devicePixelRatio ?? 1) : null)
const layoutStyle = computed(() => displaySize.value ? { '--ocr-image-width': `${displaySize.value.width + 24}px` } : {})
const ranges = computed(() => lineRanges(text.value, lines.value))
const stats = computed(() => {
  const value = text.value
  return { chars: Array.from(value.replace(/\s/g, '')).length, lines: value.trim() ? value.trim().split(/\r?\n/).length : 0 }
})
const changed = computed(() => text.value !== (props.result.fullText ?? ''))

onMounted(() => {
  if (view.value !== 'ocr') return
  void nextTick(() => editor.value?.focus())
  // 设置开启“识别后自动复制”时，结果一出现即写入剪贴板；翻译视图以译文为目的，不复制原文。
  if (props.result.autoCopy) void copy()
})

function edit(value: string) { text.value = value; message.value = ''; localError.value = '' }
function onInput(event: Event) { edit((event.target as HTMLTextAreaElement).value) }
async function copy() {
  if (busy.value || !text.value.trim()) return
  busy.value = true; message.value = ''; localError.value = ''
  try { await api.copyOcrText(text.value); message.value = '文字已复制。' }
  catch (e) { localError.value = String(e) }
  finally { busy.value = false }
}
/** 点击识别框：在文字中选中对应行并滚动到可见处，便于对照修正。 */
function selectLine(id: string) {
  activeLine.value = id
  const range = ranges.value.find(item => item.id === id)
  const input = editor.value
  if (!range || !input || view.value !== 'ocr') return
  input.focus()
  input.setSelectionRange(range.start, range.end)
  const before = text.value.slice(0, range.start).split('\n').length - 1
  const lineHeight = parseFloat(input.ownerDocument.defaultView?.getComputedStyle(input).lineHeight ?? '') || 22
  input.scrollTop = Math.max(0, before * lineHeight - input.clientHeight / 3)
}
/** 光标所在行反向点亮原图上的识别框。 */
function caretMoved() {
  const input = editor.value
  if (!input) return
  const caret = input.selectionStart
  activeLine.value = ranges.value.find(item => caret >= item.start && caret <= item.end)?.id ?? null
}
function key(event: KeyboardEvent) {
  handleEscape(event, () => emit('close'))
}
</script>

<template>
  <section class="ocr-workspace" :class="{ 'ocr-translate': view === 'translate', 'without-image': !showImage || !imageView }" :style="layoutStyle" :aria-label="view === 'translate' ? t('截图翻译') : t('文字识别结果')" @keydown.stop="key">
    <figure v-if="showImage && imageView" class="ocr-image" :aria-label="t('原图与识别区域')">
      <svg :viewBox="imageView.viewBox" :width="displaySize?.width" :height="displaySize?.height" preserveAspectRatio="xMidYMid meet" role="img" :aria-label="t('原图')">
        <image :href="imageView.url" :x="0" :y="0" :width="imageView.width" :height="imageView.height" />
        <polygon v-for="box in imageView.boxes" :key="box.id" :points="box.points" :class="{ active: activeLine === box.id }" @click="selectLine(box.id)"><title>{{ box.text }}</title></polygon>
      </svg>
      <figcaption v-if="imageView.boxes.length">{{ t('点击识别框定位文字') }}</figcaption>
    </figure>

    <div class="ocr-main">
      <template v-if="view === 'ocr'">
        <div class="ocr-tools" role="toolbar" :aria-label="t('文字整理')">
          <Button variant="ghost" :disabled="busy || !text.trim()" :title="t('合并换行')" :aria-label="t('合并换行')" @click="edit(mergeLines(text))"><WrapText :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || !text.trim()" :title="t('整理空格')" :aria-label="t('整理空格')" @click="edit(tidySpaces(text))"><Eraser :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || !changed" :title="t('还原识别结果')" :aria-label="t('还原识别结果')" @click="edit(result.fullText ?? '')"><RotateCcw :size="15" /></Button>
          <span class="ocr-tools-spacer" />
          <Button v-if="imageView" variant="ghost" :aria-pressed="showImage" :title="showImage ? t('隐藏原图') : t('显示原图')" :aria-label="showImage ? t('隐藏原图') : t('显示原图')" @click="toggleImage"><ImageIcon :size="15" /></Button>
        </div>
        <textarea ref="editor" class="ocr-editor" :value="text" :aria-label="t('识别文字，可编辑')" :disabled="busy" :placeholder="t('未识别到文字，可输入文字或重新框选。')" spellcheck="false" @input="onInput" @select="caretMoved" @click="caretMoved" @keyup="caretMoved" />
      </template>
      <TranslationPanel v-else v-model:text="text" workspace standalone :pasteable="false" />
      <p v-if="localError || error" class="ocr-error" role="alert">{{ te(localError || error) }}</p>
    </div>

    <footer class="ocr-footer">
      <template v-if="view === 'ocr'">
        <span v-if="message" class="ocr-success" role="status"><Check :size="14" />{{ t(message) }}</span>
        <span v-else class="ocr-stats">{{ t('{0} 行 · {1} 字', { 0: stats.lines, 1: stats.chars }) }}<template v-if="result.provider"> · {{ result.provider }}</template></span>
        <div class="ocr-actions">
          <Button variant="outline" :disabled="busy || !text.trim()" @click="view = 'translate'"><Languages :size="15" />{{ t('翻译文字') }}</Button>
          <Button :disabled="busy || !text.trim()" @click="copy"><Copy :size="15" />{{ t('复制文字') }}</Button>
        </div>
      </template>
      <template v-else>
        <Button variant="ghost" @click="view = 'ocr'"><ArrowLeft :size="15" />{{ t('返回识别') }}</Button>
        <Button v-if="imageView" class="ocr-image-toggle" variant="ghost" :aria-pressed="showImage" :title="showImage ? t('隐藏原图') : t('显示原图')" :aria-label="showImage ? t('隐藏原图') : t('显示原图')" @click="toggleImage"><ImageIcon :size="15" /></Button>
      </template>
    </footer>
  </section>
</template>

<style>
.ocr-workspace { --ocr-bg:#fff; --ocr-soft:#f3f5f8; --ocr-border:#e1e4e8; --ocr-text:#24292f; --ocr-muted:#6b7280; --ocr-accent:#2563eb; --ocr-box:rgba(37,99,235,.14); --ocr-box-line:rgba(37,99,235,.55); --ocr-danger:#b42318;
  display:grid; grid-template-columns:minmax(0, var(--ocr-image-width, 1fr)) minmax(320px,1fr); grid-template-rows:minmax(0,1fr) auto; grid-template-areas:'image main' 'footer footer'; width:100%; height:100%; min-height:0; background:var(--ocr-bg); color:var(--ocr-text); font-size:13px; }
[data-theme=dark] .ocr-workspace { --ocr-bg:#1b1f24; --ocr-soft:#14181d; --ocr-border:#39424e; --ocr-text:#e6edf3; --ocr-muted:#9aa7b4; --ocr-accent:#79a6ff; --ocr-box:rgba(121,166,255,.16); --ocr-box-line:rgba(121,166,255,.6); --ocr-danger:#ff8a80; }
.ocr-workspace.without-image { grid-template-columns:minmax(0,1fr); grid-template-areas:'main' 'footer'; }
.ocr-image { grid-area:image; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:6px; min-width:0; min-height:0; margin:0; padding:12px; background:var(--ocr-soft); border-right:1px solid var(--ocr-border); }
.ocr-image svg { flex:none; max-width:100%; max-height:calc(100% - 24px); height:auto; }
.ocr-image polygon { fill:transparent; stroke:var(--ocr-box-line); stroke-width:1; vector-effect:non-scaling-stroke; cursor:pointer; }
.ocr-image polygon:hover { fill:var(--ocr-box); }
.ocr-image polygon.active { fill:var(--ocr-box); stroke:var(--ocr-accent); stroke-width:2; }
.ocr-image figcaption { color:var(--ocr-muted); font-size:11px; text-align:center; }
.ocr-main { grid-area:main; display:flex; flex-direction:column; min-width:0; min-height:0; }
.ocr-tools { display:flex; align-items:center; gap:2px; padding:6px 8px; border-bottom:1px solid var(--ocr-border); }
.ocr-tools button, .ocr-footer .ocr-image-toggle { width:30px; height:30px; padding:0; color:var(--ocr-muted); }
.ocr-tools button[aria-pressed=true], .ocr-footer .ocr-image-toggle[aria-pressed=true] { color:var(--ocr-accent); }
.ocr-tools-spacer { flex:1; }
.ocr-editor { flex:1; width:100%; min-height:0; padding:12px 16px; border:0; outline:none; resize:none; background:transparent; color:var(--ocr-text); font:14px/1.75 'Segoe UI','Microsoft YaHei',sans-serif; user-select:text; }
.ocr-editor::placeholder { color:var(--ocr-muted); }
.ocr-error { margin:0 12px 8px; padding:6px 10px; border-radius:6px; background:rgba(180,35,24,.08); color:var(--ocr-danger); font-size:12px; overflow-wrap:anywhere; }
.ocr-footer { grid-area:footer; display:flex; align-items:center; gap:8px; min-height:52px; padding:8px 12px; border-top:1px solid var(--ocr-border); }
.ocr-footer > button { height:34px; padding:0 12px; }
.ocr-stats { min-width:0; overflow:hidden; color:var(--ocr-muted); font-size:12px; text-overflow:ellipsis; white-space:nowrap; }
.ocr-success { display:inline-flex; align-items:center; gap:4px; color:var(--ui-success-fg, #24834b); font-size:12px; }
.ocr-actions { display:flex; gap:8px; margin-left:auto; }
.ocr-actions button { height:34px; padding:0 14px; }
.ocr-footer .ocr-image-toggle { margin-left:auto; }
/* 就地翻译：翻译面板填满右侧，原文与译文按比例分高，不再使用浮窗的固定高度。 */
.ocr-workspace.ocr-translate .ocr-main { padding:10px; background:var(--ocr-soft); }
.ocr-workspace.ocr-translate .translation-panel { flex:1; min-height:0; }
.ocr-workspace.ocr-translate .translation-panel .source-card { display:flex; flex:1 1 40%; flex-direction:column; min-height:120px; }
.ocr-workspace.ocr-translate .translation-panel .provider-card { display:flex; flex:1 1 60%; flex-direction:column; min-height:140px; }
.ocr-workspace.ocr-translate .translation-panel .source-card textarea, .ocr-workspace.ocr-translate .translation-panel .provider-card textarea { flex:1; height:auto; min-height:0; max-height:none; resize:none; }
@media (max-width: 640px) {
  .ocr-workspace { grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(120px,38%) minmax(0,1fr) auto; grid-template-areas:'image' 'main' 'footer'; }
  .ocr-image { border-right:0; border-bottom:1px solid var(--ocr-border); }
}
</style>
