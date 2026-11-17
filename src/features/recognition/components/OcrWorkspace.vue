<script setup lang="ts">
import { computed, nextTick, onMounted, ref, shallowRef, watch } from 'vue'
import { ArrowLeft, Check, Copy, Eraser, Eye, EyeOff, Image as ImageIcon, Languages, RotateCcw, WrapText } from 'lucide-vue-next'
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { api } from '@/services/api'
import Button from '@/shared/ui/button/Button.vue'
import TranslationPanel from '@/features/recognition/components/TranslationPanel.vue'
import type { OcrResult } from '@/features/recognition/ai'
import { lineRanges, mergeLines, ocrDisplaySize, ocrImageView, tidySpaces, type OcrImage } from '@/features/recognition/ocr-workspace'
import { fitText, LINE_SPACING, overlayBoxes, overlayParagraphs, sampleColors, type OverlayColors } from '@/features/recognition/translation-overlay'

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
const layoutStyle = computed(() => ({
  ...(displaySize.value ? { '--ocr-image-width': `${displaySize.value.width + 24}px` } : {}),
  // 覆盖模式只留原图一列；用内联样式而非类选择器，独立窗口同步样式表滞后时也能生效。
  ...(overlayOnly.value ? { gridTemplateColumns: 'minmax(0,1fr)', gridTemplateAreas: "'image' 'footer'" } : {}),
}))
const ranges = computed(() => lineRanges(text.value, lines.value))
const stats = computed(() => {
  const value = text.value
  return { chars: Array.from(value.replace(/\s/g, '')).length, lines: value.trim() ? value.trim().split(/\r?\n/).length : 0 }
})
const changed = computed(() => text.value !== (props.result.fullText ?? ''))

/**
 * 译文覆盖原文（翻译设置开启时）：翻译面板按段落回传译文，这里取原图配色后把译文画回原位置。
 * 任何一步不满足（无坐标、倾斜、原文已改、取色失败）都只显示普通翻译结果。
 */
const paragraphs = computed(() => overlayParagraphs(lines.value))
const segments = computed(() => paragraphs.value?.map(item => item.text) ?? null)
const overlayTexts = shallowRef<string[] | null>(null)
const overlayColors = shallowRef<OverlayColors[] | null>(null)
const showOverlay = ref(true)
const overlayFont = "'Segoe UI','Microsoft YaHei','PingFang SC',sans-serif"
let measureContext: CanvasRenderingContext2D | null | undefined
function measure(value: string, size: number) {
  measureContext ??= document.createElement('canvas').getContext('2d')
  if (!measureContext) return value.length * size * 0.6
  measureContext.font = `${size}px ${overlayFont}`
  return measureContext.measureText(value).width
}
const overlayAvailable = computed(() => !!props.image && !!overlayTexts.value && !!overlayColors.value)
const overlay = computed(() => {
  const image = props.image, list = paragraphs.value, texts = overlayTexts.value, colors = overlayColors.value
  if (view.value !== 'translate' || !showOverlay.value || !image || !list || !texts || !colors || texts.length !== list.length || colors.length !== list.length) return null
  const boxes = overlayBoxes(image, props.result, list)
  if (!boxes) return null
  return boxes.map((box, index) => {
    const scale = list[index].box.width > 0 ? box.width / list[index].box.width : 1
    const lineHeight = list[index].lineHeight * scale
    const layout = fitText(texts[index], box, lineHeight, measure)
    const pad = Math.max(1, lineHeight * 0.08)
    const top = box.y + Math.max(0, (box.height - layout.lines.length * layout.fontSize * LINE_SPACING) / 2)
    return { id: list[index].ids[0], box, pad, top, ...colors[index], ...layout }
  })
})
/** 只读取当前段落附近的原图像素取色；图片按显示坐标绘制，迟到的结果按引用比较后丢弃。 */
async function loadColors(texts: string[] | null) {
  overlayColors.value = null
  const image = props.image, list = paragraphs.value
  if (!texts || !image || !list) return
  const boxes = overlayBoxes(image, props.result, list)
  if (!boxes) return
  try {
    const element = new Image()
    element.src = image.url
    await element.decode()
    const crop = image.crop ?? { x: 0, y: 0, width: image.width, height: image.height }
    const fx = element.naturalWidth / Math.max(1, image.width), fy = element.naturalHeight / Math.max(1, image.height)
    const width = Math.max(1, Math.round(crop.width)), height = Math.max(1, Math.round(crop.height))
    const canvas = document.createElement('canvas')
    canvas.width = width; canvas.height = height
    const context = canvas.getContext('2d', { willReadFrequently: true })
    if (!context) return
    context.drawImage(element, crop.x * fx, crop.y * fy, crop.width * fx, crop.height * fy, 0, 0, width, height)
    const pixels = context.getImageData(0, 0, width, height)
    if (overlayTexts.value !== texts) return
    overlayColors.value = boxes.map(box => sampleColors(pixels, { ...box, x: box.x - crop.x, y: box.y - crop.y }))
  } catch { if (overlayTexts.value === texts) overlayColors.value = null }
}
watch(overlayTexts, texts => { void loadColors(texts) })
/**
 * 覆盖模式下右侧翻译面板只负责发起翻译，不再显示：null 表示面板尚未读到翻译设置，暂时隐身以免闪烁；
 * 面板出错或退回普通翻译时报告 false，面板随即显示。隐藏原图时面板必须可见。
 */
const overlayMode = ref<boolean | null>(null)
const translating = ref(false)
watch(view, value => { if (value === 'translate') overlayMode.value = null })
const overlayOnly = computed(() => view.value === 'translate' && overlayMode.value === true && showImage.value && !!imageView.value)
async function copyTranslation() {
  const value = overlayTexts.value?.join('\n\n') ?? ''
  if (busy.value || !value.trim()) return
  busy.value = true; message.value = ''; localError.value = ''
  try { await api.copyOcrText(value); message.value = '译文已复制。' }
  catch (e) { localError.value = String(e) }
  finally { busy.value = false }
}

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
  <section class="ocr-workspace" :class="{ 'ocr-translate': view === 'translate', 'without-image': !showImage || !imageView, 'overlay-only': overlayOnly }" :style="layoutStyle" :aria-label="view === 'translate' ? t('截图翻译') : t('文字识别结果')" @keydown.stop="key">
    <figure v-if="showImage && imageView" class="ocr-image" :aria-label="t('原图与识别区域')">
      <svg :viewBox="imageView.viewBox" :width="displaySize?.width" :height="displaySize?.height" preserveAspectRatio="xMidYMid meet" role="img" :aria-label="t('原图')">
        <image :href="imageView.url" :x="0" :y="0" :width="imageView.width" :height="imageView.height" />
        <polygon v-for="box in (overlay ? [] : imageView.boxes)" :key="box.id" :points="box.points" :class="{ active: activeLine === box.id }" @click="selectLine(box.id)"><title>{{ box.text }}</title></polygon>
        <g v-if="overlay" class="ocr-overlay" :style="{ fontFamily: overlayFont }">
          <g v-for="item in overlay" :key="item.id">
            <rect :x="item.box.x - item.pad" :y="item.box.y - item.pad" :width="item.box.width + item.pad * 2" :height="item.box.height + item.pad * 2" :fill="item.background" />
            <text :fill="item.foreground" :font-size="item.fontSize" dominant-baseline="central">
              <tspan v-for="(line, index) in item.lines" :key="index" :x="item.box.x" :y="item.top + item.fontSize * LINE_SPACING * (index + 0.5)">{{ line }}</tspan>
            </text>
          </g>
        </g>
      </svg>
      <figcaption v-if="imageView.boxes.length && !overlay">{{ t('点击识别框定位文字') }}</figcaption>
    </figure>

    <div v-show="!overlayOnly" class="ocr-main" :style="view === 'translate' && overlayMode === null ? { visibility: 'hidden' } : undefined">
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
      <TranslationPanel v-else v-model:text="text" workspace standalone :pasteable="false" :segments="segments" @segments="overlayTexts = $event" @overlay="overlayMode = $event" @busy="translating = $event" />
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
        <template v-if="overlayOnly">
          <span v-if="localError || error" class="ocr-footer-error" role="alert">{{ te(localError || error) }}</span>
          <span v-else-if="message" class="ocr-success" role="status"><Check :size="14" />{{ t(message) }}</span>
          <span v-else-if="translating" class="ocr-stats" role="status">{{ t('正在翻译…') }}</span>
        </template>
        <Button v-if="imageView" class="ocr-image-toggle" variant="ghost" :aria-pressed="showImage" :title="showImage ? t('隐藏原图') : t('显示原图')" :aria-label="showImage ? t('隐藏原图') : t('显示原图')" @click="toggleImage"><ImageIcon :size="15" /></Button>
        <Button v-if="overlayAvailable && showImage" class="ocr-overlay-toggle" variant="ghost" :aria-pressed="showOverlay" :title="showOverlay ? t('显示原文') : t('显示译文')" :aria-label="showOverlay ? t('显示原文') : t('显示译文')" @click="showOverlay = !showOverlay"><Eye v-if="showOverlay" :size="15" /><EyeOff v-else :size="15" /></Button>
        <Button v-if="overlayOnly" class="ocr-overlay-copy" :disabled="busy || !overlayTexts" @click="copyTranslation"><Copy :size="15" />{{ t('复制译文') }}</Button>
      </template>
    </footer>
  </section>
</template>

<style>
.ocr-workspace { --ocr-bg:#fff; --ocr-soft:#f3f5f8; --ocr-border:#e1e4e8; --ocr-text:#24292f; --ocr-muted:#6b7280; --ocr-accent:#2563eb; --ocr-box:rgba(37,99,235,.14); --ocr-box-line:rgba(37,99,235,.55); --ocr-danger:#b42318;
  display:grid; grid-template-columns:minmax(0, var(--ocr-image-width, 1fr)) minmax(320px,1fr); grid-template-rows:minmax(0,1fr) auto; grid-template-areas:'image main' 'footer footer'; width:100%; height:100%; min-height:0; background:var(--ocr-bg); color:var(--ocr-text); font-size:13px; }
[data-theme=dark] .ocr-workspace { --ocr-bg:#1b1f24; --ocr-soft:#14181d; --ocr-border:#39424e; --ocr-text:#e6edf3; --ocr-muted:#9aa7b4; --ocr-accent:#79a6ff; --ocr-box:rgba(121,166,255,.16); --ocr-box-line:rgba(121,166,255,.6); --ocr-danger:#ff8a80; }
.ocr-workspace.without-image { grid-template-columns:minmax(0,1fr); grid-template-areas:'main' 'footer'; }
.ocr-workspace.overlay-only .ocr-image { border-right:0; }
.ocr-footer-error { min-width:0; overflow:hidden; color:var(--ocr-danger); font-size:12px; text-overflow:ellipsis; white-space:nowrap; }
.ocr-footer .ocr-overlay-copy { height:34px; padding:0 14px; }
.ocr-image { grid-area:image; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:6px; min-width:0; min-height:0; margin:0; padding:12px; background:var(--ocr-soft); border-right:1px solid var(--ocr-border); }
.ocr-image svg { flex:none; max-width:100%; max-height:calc(100% - 24px); height:auto; }
.ocr-image polygon { fill:transparent; stroke:var(--ocr-box-line); stroke-width:1; vector-effect:non-scaling-stroke; cursor:pointer; }
.ocr-image polygon:hover { fill:var(--ocr-box); }
.ocr-image polygon.active { fill:var(--ocr-box); stroke:var(--ocr-accent); stroke-width:2; }
.ocr-overlay { pointer-events:none; white-space:pre; }
.ocr-image figcaption { color:var(--ocr-muted); font-size:11px; text-align:center; }
.ocr-main { grid-area:main; display:flex; flex-direction:column; min-width:0; min-height:0; }
.ocr-tools { display:flex; align-items:center; gap:2px; padding:6px 8px; border-bottom:1px solid var(--ocr-border); }
.ocr-tools button, .ocr-footer .ocr-image-toggle, .ocr-footer .ocr-overlay-toggle { width:30px; height:30px; padding:0; color:var(--ocr-muted); }
.ocr-tools button[aria-pressed=true], .ocr-footer .ocr-image-toggle[aria-pressed=true], .ocr-footer .ocr-overlay-toggle[aria-pressed=true] { color:var(--ocr-accent); }
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
