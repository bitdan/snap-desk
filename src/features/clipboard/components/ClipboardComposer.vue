<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { FileImage, FileText, GripVertical, ImageIcon, LoaderCircle, Pencil, Type, Undo2, X } from 'lucide-vue-next'
import { t, te } from '@/shared/i18n/i18n'
import { api } from '@/services/api'
import Button from '@/shared/ui/button/Button.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { handleEscape } from '@/shared/ui/escape'
import { clipboardThumbnails } from '@/features/clipboard/clipboard-thumbnails'
import type { ClipboardExportOptions, ClipboardItem } from '@/features/clipboard/clipboard'
import {
  clamp, createEntries, edited, entryText, exportPayload, invalidEntry, loadComposerOptions, longPngSize, moveEntry,
  pdfPages, saveComposerOptions, separateNames, LONG_PNG_MAX_PIXELS, PDF_FONT_MM, PDF_LINE_MM,
  PDF_MAX_PAGES, PDF_MAX_PIXELS, type ComposerEntry, type ComposerMode,
} from '@/features/clipboard/clipboard-composer'

/**
 * 组合导出编辑器：左侧调整素材顺序、移除或改写文本，右侧按当前参数实时预览。
 * 改写只作用于本次导出，不修改历史；关闭后素材列表随组件一起丢弃。
 */
const props = defineProps<{ items: ClipboardItem[]; busy?: boolean; error?: string; message?: string }>()
const emit = defineEmits<{ export: [options: ClipboardExportOptions]; close: [] }>()

const entries = ref<ComposerEntry[]>(createEntries(props.items))
const options = ref(loadComposerOptions())
const thumbnails = ref<Record<number, string>>({})
const editing = ref<number | null>(null), loadError = ref('')
const dragging = ref<number | null>(null)
const controller = new AbortController()

watch(options, value => saveComposerOptions(value), { deep: true })

const modes: { value: ComposerMode; label: string }[] = [
  { value: 'pdf', label: '多页 PDF' }, { value: 'longPng', label: '纵向长图' }, { value: 'separate', label: '分别导出' },
]
const pages = computed(() => options.value.mode === 'pdf' ? pdfPages(entries.value, options.value) : [])
const longSize = computed(() => options.value.mode === 'longPng' ? longPngSize(entries.value, options.value) : { width: 0, height: 0 })
const names = computed(() => separateNames(entries.value))
const invalid = computed(() => invalidEntry(entries.value))
/** 预览阶段就提示后端会拒绝的规模，避免选好保存位置后才失败。 */
const limitWarning = computed(() => {
  if (options.value.mode === 'pdf') {
    const pixels = entries.value.reduce((sum, entry) => entry.kind === 'image' ? sum + entry.width * entry.height : sum, 0)
    if (pixels > PDF_MAX_PIXELS) return t('PDF 图片总量超过 6400 万像素，请分批导出。')
    if (pages.value.length > PDF_MAX_PAGES) return t('PDF 超过 1000 页，请分批导出。')
  }
  if (options.value.mode === 'longPng' && longSize.value.width * longSize.value.height > LONG_PNG_MAX_PIXELS) return t('长图超过 3200 万像素，请减少素材或改用 PDF。')
  return ''
})
const summary = computed(() => {
  const count = t('{0} 个素材', { 0: entries.value.length })
  if (options.value.mode === 'pdf') return `${count} · ${t('约 {0} 页', { 0: pages.value.length })}`
  if (options.value.mode === 'longPng') return `${count} · ${t('约 {0} × {1} px', { 0: longSize.value.width, 1: longSize.value.height })}`
  return `${count} · ${t('{0} 个文件', { 0: entries.value.length })}`
})
const canExport = computed(() => !props.busy && entries.value.length > 0 && invalid.value < 0 && !limitWarning.value)

/** 文本取完整原文用于改写和预览；图片只取缩略图，导出时后端仍使用原图。 */
onMounted(() => {
  for (const entry of entries.value) {
    const id = entry.id
    if (entry.kind === 'image') {
      void clipboardThumbnails.get(id, controller.signal)
        .then(value => { thumbnails.value = { ...thumbnails.value, [id]: value } })
        .catch(() => { /* 缩略图失败只影响预览，不阻止导出。 */ })
    } else {
      void api.clipboardText(id)
        .then(value => { const target = entries.value.find(item => item.id === id); if (target) target.original = value })
        .catch(e => { if (!controller.signal.aborted) loadError.value = String(e) })
    }
  }
})
onUnmounted(() => controller.abort())

function setMode(mode: ComposerMode) { options.value.mode = mode }
function setNumber(key: 'gapPx' | 'marginMm', event: Event, min: number, max: number) {
  const input = event.target as HTMLInputElement
  options.value[key] = clamp(input.valueAsNumber, min, max, options.value[key])
}
function remove(index: number) {
  if (editing.value === entries.value[index]?.id) editing.value = null
  entries.value = entries.value.filter((_, i) => i !== index)
}
function move(from: number, to: number) { entries.value = moveEntry(entries.value, from, to) }
function edit(entry: ComposerEntry) {
  if (entry.original === null) return
  editing.value = editing.value === entry.id ? null : entry.id
}
function updateText(entry: ComposerEntry, event: Event) { entry.text = (event.target as HTMLTextAreaElement).value }
function restore(entry: ComposerEntry) { entry.text = undefined }

/** 卡片键盘：Alt+↑/↓ 调整顺序，Delete 移除；焦点跟随卡片移动。 */
function cardKey(event: KeyboardEvent, index: number) {
  if ((event.target as HTMLElement).closest('textarea, button')) return
  const doc = (event.currentTarget as HTMLElement).ownerDocument
  const focus = (target: number) => void nextTick(() => doc.querySelector<HTMLElement>(`[data-composer-index="${target}"]`)?.focus())
  if (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
    const target = index + (event.key === 'ArrowUp' ? -1 : 1)
    if (target < 0 || target >= entries.value.length) return
    event.preventDefault(); move(index, target); focus(target)
  } else if (event.key === 'Delete') {
    event.preventDefault(); remove(index); focus(Math.min(index, entries.value.length - 1))
  }
}
/** 手柄拖动用指针事件实现：WebView2 的原生拖放会被 Tauri 文件拖入处理拦截。 */
function dragStart(event: PointerEvent, index: number) {
  if (event.button !== 0 || props.busy) return
  event.preventDefault()
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  dragging.value = index
}
function dragMove(event: PointerEvent) {
  if (dragging.value === null) return
  const doc = (event.currentTarget as HTMLElement).ownerDocument
  const card = doc.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-composer-index]')
  const target = card ? Number(card.dataset.composerIndex) : NaN
  if (Number.isInteger(target) && target !== dragging.value) { move(dragging.value, target); dragging.value = target }
}
function dragEnd() { dragging.value = null }

function submit() {
  if (!canExport.value) return
  editing.value = null
  emit('export', exportPayload(entries.value, options.value))
}

/** 预览尺寸都换算成相对页宽 / 图宽的百分比，窗口缩放时版式保持比例。 */
function pageStyle(width: number, height: number) {
  return { '--page-ratio': `${width} / ${height}`, '--page-scale': Math.min(1.6, width / 210), '--page-pad': `${options.value.marginMm / width * 100}%` }
}
const pdfTextStyle = computed(() => ({ fontSize: `${PDF_FONT_MM / 210 * 100}cqw`, lineHeight: `${PDF_LINE_MM / 210 * 100}cqw` }))
/** 后端把每块（含 1200px 宽的文本图）都缩放到长图全宽，预览同样占满宽度。 */
function longBlockStyle(entry: ComposerEntry) {
  return entry.kind === 'image' ? { aspectRatio: `${entry.width} / ${entry.height}` } : {}
}
const longGap = computed(() => `${options.value.gapPx / Math.max(1, longSize.value.width) * 100}cqw`)
</script>

<template>
  <section class="clip-composer" :aria-busy="busy" @keydown.stop="handleEscape($event, () => { if (!busy) emit('close') })">
    <header class="composer-toolbar">
      <div class="composer-modes" role="radiogroup" :aria-label="t('导出方式')">
        <button v-for="mode in modes" :key="mode.value" type="button" role="radio" :aria-checked="options.mode === mode.value" :disabled="busy" @click="setMode(mode.value)">{{ t(mode.label) }}</button>
      </div>
      <div class="composer-params">
        <template v-if="options.mode === 'pdf'">
          <SelectControl v-model="options.pageSize" :disabled="busy" :aria-label="t('页面大小')" :options="[{ value: 'A4', label: 'A4' }, { value: 'original', label: t('图片原始比例') }]" />
          <label>{{ t('边距') }}<input type="number" min="0" max="30" :value="options.marginMm" :disabled="busy" @change="setNumber('marginMm', $event, 0, 30)" />mm</label>
        </template>
        <template v-else-if="options.mode === 'longPng'">
          <label>{{ t('间距') }}<input type="number" min="0" max="200" :value="options.gapPx" :disabled="busy" @change="setNumber('gapPx', $event, 0, 200)" />px</label>
          <SelectControl v-model="options.background" :disabled="busy" :aria-label="t('背景')" :options="[{ value: 'white', label: t('白底') }, { value: 'transparent', label: t('透明背景') }]" />
        </template>
        <span v-else class="composer-hint">{{ t('文本存为 TXT，图片按原图存为 PNG，放进新建的批次文件夹。') }}</span>
      </div>
    </header>

    <div class="composer-body">
      <aside class="composer-list" :aria-label="t('素材顺序')">
        <p class="composer-list-hint">{{ t('拖动手柄或 Alt+↑/↓ 调整顺序，改写只影响本次导出。') }}</p>
        <ol>
          <li v-for="(entry, index) in entries" :key="entry.id" class="composer-card" :class="{ dragging: dragging === index, editing: editing === entry.id, invalid: invalid === index }" :data-composer-index="index" tabindex="0" :aria-label="t('第 {0} 项', { 0: index + 1 })" @keydown="cardKey($event, index)">
            <div class="composer-card-head">
              <button type="button" class="composer-grip" :aria-label="t('拖动调整顺序')" :disabled="busy" @pointerdown="dragStart($event, index)" @pointermove="dragMove" @pointerup="dragEnd" @pointercancel="dragEnd" @lostpointercapture="dragEnd"><GripVertical :size="14" aria-hidden="true" /></button>
              <span class="composer-index">{{ index + 1 }}</span>
              <span class="composer-kind"><Type v-if="entry.kind === 'text'" :size="13" aria-hidden="true" /><ImageIcon v-else :size="13" aria-hidden="true" />{{ entry.kind === 'text' ? t('文本') : `${entry.width} × ${entry.height}` }}</span>
              <span v-if="edited(entry)" class="composer-badge">{{ t('已改写') }}</span>
              <span class="composer-card-actions">
                <button v-if="edited(entry)" type="button" :title="t('还原原文')" :aria-label="t('还原原文')" :disabled="busy" @click="restore(entry)"><Undo2 :size="14" aria-hidden="true" /></button>
                <button v-if="entry.kind === 'text'" type="button" :title="t('改写文本')" :aria-label="t('改写文本')" :aria-pressed="editing === entry.id" :disabled="busy || entry.original === null" @click="edit(entry)"><Pencil :size="14" aria-hidden="true" /></button>
                <button type="button" class="composer-remove" :title="t('从本次导出移除')" :aria-label="t('从本次导出移除')" :disabled="busy" @click="remove(index)"><X :size="14" aria-hidden="true" /></button>
              </span>
            </div>
            <textarea v-if="editing === entry.id" class="composer-editor" :value="entryText(entry)" :aria-label="t('改写文本')" spellcheck="false" :disabled="busy" @input="updateText(entry, $event)" />
            <p v-else-if="entry.kind === 'text'" class="composer-snippet">{{ entryText(entry) }}</p>
            <div v-else class="composer-thumb"><img v-if="thumbnails[entry.id]" :src="thumbnails[entry.id]" alt="" draggable="false" /></div>
            <p v-if="invalid === index" class="composer-card-error" role="alert">{{ t('改写后的文本不能为空。') }}</p>
          </li>
        </ol>
        <p v-if="!entries.length" class="composer-empty">{{ t('素材已全部移除，关闭后重新选择。') }}</p>
      </aside>

      <main class="composer-preview" :class="`mode-${options.mode}`" :aria-label="t('导出预览')">
        <template v-if="options.mode === 'pdf'">
          <figure v-for="(page, number) in pages" :key="number" class="composer-page" :style="pageStyle(page.width, page.height)">
            <div class="composer-page-sheet">
              <div v-if="page.kind === 'text'" class="composer-page-text" :style="pdfTextStyle"><div v-for="(line, i) in page.lines" :key="i">{{ line || ' ' }}</div></div>
              <img v-else-if="thumbnails[entries[page.entry].id]" :src="thumbnails[entries[page.entry].id]" alt="" draggable="false" />
            </div>
            <figcaption>{{ t('第 {0} 页', { 0: number + 1 }) }} · {{ t('素材 {0}', { 0: page.entry + 1 }) }}</figcaption>
          </figure>
        </template>
        <div v-else-if="options.mode === 'longPng'" class="composer-long" :class="{ transparent: options.background === 'transparent' }" :style="{ gap: longGap }">
          <div v-for="entry in entries" :key="entry.id" class="composer-long-block" :class="entry.kind" :style="longBlockStyle(entry)">
            <p v-if="entry.kind === 'text'">{{ entryText(entry) }}</p>
            <img v-else-if="thumbnails[entry.id]" :src="thumbnails[entry.id]" alt="" draggable="false" />
          </div>
        </div>
        <ul v-else class="composer-files">
          <li v-for="(entry, index) in entries" :key="entry.id">
            <div class="composer-file-thumb"><img v-if="entry.kind === 'image' && thumbnails[entry.id]" :src="thumbnails[entry.id]" alt="" draggable="false" /><p v-else-if="entry.kind === 'text'">{{ entryText(entry) }}</p></div>
            <span><FileText v-if="entry.kind === 'text'" :size="13" aria-hidden="true" /><FileImage v-else :size="13" aria-hidden="true" />{{ names[index] }}</span>
          </li>
        </ul>
        <p class="composer-preview-note">{{ t('预览为近似排版，文字换行以导出结果为准。') }}</p>
      </main>
    </div>

    <footer class="composer-footer">
      <div class="composer-status" role="status">
        <span v-if="error || loadError" class="composer-error">{{ te(error || loadError) }}</span>
        <span v-else-if="limitWarning" class="composer-error">{{ limitWarning }}</span>
        <span v-else-if="message" class="composer-message">{{ t(message) }}</span>
        <span v-else>{{ summary }}</span>
      </div>
      <Button variant="outline" :disabled="busy" @click="emit('close')">{{ t('关闭') }}</Button>
      <Button :disabled="!canExport" @click="submit"><LoaderCircle v-if="busy" :size="14" class="composer-spinner" aria-hidden="true" />{{ busy ? t('正在导出…') : t('选择位置并导出') }}</Button>
    </footer>
  </section>
</template>

<style>
.clip-composer { --cmp-bg:#fff; --cmp-soft:#f3f5f8; --cmp-card:#fff; --cmp-border:#e1e4e8; --cmp-text:#24292f; --cmp-muted:#6b7280; --cmp-accent:#2563eb; --cmp-selected:#eff6ff; --cmp-danger:#b42318; --cmp-paper:#fff; display:grid; grid-template-rows:auto minmax(0,1fr) auto; height:100%; min-height:0; background:var(--cmp-bg); color:var(--cmp-text); font-size:13px; }
[data-theme=dark] .clip-composer { --cmp-bg:#1b1f24; --cmp-soft:#14181d; --cmp-card:#252b33; --cmp-border:#39424e; --cmp-text:#e6edf3; --cmp-muted:#9aa7b4; --cmp-accent:#79a6ff; --cmp-selected:#1f3352; --cmp-danger:#ff8a80; }
.composer-toolbar { display:flex; flex-wrap:wrap; align-items:center; gap:8px 16px; padding:10px 14px; border-bottom:1px solid var(--cmp-border); }
.composer-modes { display:inline-flex; padding:2px; border-radius:8px; background:var(--cmp-soft); }
.composer-modes button { height:28px; padding:0 12px; border-radius:6px; color:var(--cmp-muted); white-space:nowrap; }
.composer-modes button[aria-checked=true] { background:var(--cmp-bg); color:var(--cmp-accent); font-weight:600; box-shadow:0 1px 2px rgba(0,0,0,.08); }
.composer-modes button:focus-visible, .composer-card button:focus-visible, .composer-card:focus-visible { outline:2px solid var(--cmp-accent); outline-offset:-2px; }
.composer-params { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; min-width:0; }
.composer-params label { display:inline-flex; align-items:center; gap:6px; color:var(--cmp-muted); white-space:nowrap; }
.composer-params input { width:64px; height:30px; padding:0 6px; border:1px solid var(--cmp-border); border-radius:6px; background:var(--cmp-bg); color:var(--cmp-text); }
.composer-hint { color:var(--cmp-muted); font-size:12px; }
.composer-body { display:grid; grid-template-columns:minmax(240px, 320px) minmax(0,1fr); min-height:0; }
.composer-list { min-height:0; padding:10px; overflow-y:auto; border-right:1px solid var(--cmp-border); }
.composer-list-hint, .composer-empty { margin:0 2px 8px; color:var(--cmp-muted); font-size:12px; line-height:1.5; }
.composer-list ol { display:flex; flex-direction:column; gap:6px; margin:0; padding:0; list-style:none; }
.composer-card { padding:6px 8px 8px; border:1px solid var(--cmp-border); border-radius:8px; background:var(--cmp-card); }
.composer-card.dragging { border-color:var(--cmp-accent); box-shadow:0 4px 12px rgba(37,99,235,.18); }
.composer-card.editing { border-color:var(--cmp-accent); }
.composer-card.invalid { border-color:var(--cmp-danger); }
.composer-card-head { display:flex; align-items:center; gap:6px; min-height:24px; }
.composer-grip { display:flex; align-items:center; justify-content:center; width:20px; height:24px; margin-left:-4px; border-radius:4px; color:var(--cmp-muted); cursor:grab; touch-action:none; }
.composer-card.dragging .composer-grip { cursor:grabbing; }
.composer-index { min-width:18px; height:18px; padding:0 4px; border-radius:9px; background:var(--cmp-selected); color:var(--cmp-accent); font-size:11px; font-weight:600; line-height:18px; text-align:center; }
.composer-kind { display:inline-flex; align-items:center; gap:4px; color:var(--cmp-muted); font-size:12px; }
.composer-badge { padding:0 6px; border-radius:4px; background:var(--cmp-selected); color:var(--cmp-accent); font-size:11px; }
.composer-card-actions { display:flex; gap:2px; margin-left:auto; }
.composer-card-actions button { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:4px; color:var(--cmp-muted); }
.composer-card-actions button:hover:enabled { background:var(--cmp-soft); color:var(--cmp-text); }
.composer-card-actions button[aria-pressed=true] { background:var(--cmp-selected); color:var(--cmp-accent); }
.composer-card-actions .composer-remove:hover:enabled { color:var(--cmp-danger); }
.composer-snippet { display:-webkit-box; margin:4px 0 0; overflow:hidden; color:var(--cmp-text); font-size:12px; line-height:1.5; white-space:pre-wrap; overflow-wrap:anywhere; -webkit-line-clamp:3; -webkit-box-orient:vertical; }
.composer-editor { display:block; width:100%; min-height:120px; margin-top:6px; padding:6px 8px; resize:vertical; border:1px solid var(--cmp-border); border-radius:6px; background:var(--cmp-bg); color:var(--cmp-text); font:12px/1.6 'Segoe UI','Microsoft YaHei',sans-serif; }
.composer-editor:focus { outline:2px solid var(--cmp-accent); outline-offset:-1px; }
.composer-thumb { display:flex; justify-content:center; height:72px; margin-top:4px; border-radius:4px; background:var(--cmp-soft); overflow:hidden; }
.composer-thumb img { max-width:100%; max-height:100%; object-fit:contain; }
.composer-card-error { margin:4px 0 0; color:var(--cmp-danger); font-size:12px; }
.composer-preview { display:flex; flex-direction:column; align-items:center; gap:18px; min-height:0; padding:20px; overflow:auto; background:var(--cmp-soft); }
.composer-page { width:min(100%, calc(320px * var(--page-scale))); margin:0; }
.composer-page-sheet { container-type:inline-size; display:flex; align-items:center; justify-content:center; aspect-ratio:var(--page-ratio); padding:var(--page-pad); box-sizing:border-box; background:var(--cmp-paper); box-shadow:0 1px 4px rgba(0,0,0,.18); overflow:hidden; }
.composer-page-text { align-self:flex-start; width:100%; color:#111; white-space:pre; overflow:hidden; font-family:'Noto Sans SC','Microsoft YaHei',sans-serif; }
.composer-page-sheet img { max-width:100%; max-height:100%; object-fit:contain; }
.composer-page figcaption { margin-top:6px; color:var(--cmp-muted); font-size:11px; text-align:center; }
.composer-long { container-type:inline-size; display:flex; flex-direction:column; align-items:stretch; width:min(100%, 420px); background:#fff; box-shadow:0 1px 4px rgba(0,0,0,.18); }
.composer-long.transparent { background:repeating-conic-gradient(#e6e6e6 0 25%, #fff 0 50%) 0 0 / 16px 16px; }
.composer-long-block { width:100%; }
.composer-long-block.image img { display:block; width:100%; height:100%; object-fit:fill; }
.composer-long-block.text { container-type:inline-size; background:#fff; }
.composer-long-block.text p { margin:0; padding:2cqw; color:#111; font-size:1.38cqw; line-height:2.83cqw; white-space:pre-wrap; overflow-wrap:anywhere; font-family:'Noto Sans SC','Microsoft YaHei',sans-serif; }
.composer-files { display:grid; grid-template-columns:repeat(auto-fill, minmax(120px, 1fr)); gap:12px; width:100%; margin:0; padding:0; list-style:none; }
.composer-files li { display:flex; flex-direction:column; gap:6px; min-width:0; }
.composer-file-thumb { display:flex; align-items:center; justify-content:center; aspect-ratio:4 / 3; border:1px solid var(--cmp-border); border-radius:6px; background:var(--cmp-bg); overflow:hidden; }
.composer-file-thumb img { max-width:100%; max-height:100%; object-fit:contain; }
.composer-file-thumb p { display:-webkit-box; margin:0; padding:8px; overflow:hidden; color:var(--cmp-text); font-size:11px; line-height:1.4; white-space:pre-wrap; overflow-wrap:anywhere; -webkit-line-clamp:5; -webkit-box-orient:vertical; }
.composer-files span { display:inline-flex; align-items:center; gap:4px; color:var(--cmp-muted); font-size:12px; }
.composer-preview-note { margin:0; color:var(--cmp-muted); font-size:11px; }
.composer-footer { display:flex; align-items:center; gap:8px; padding:10px 14px; border-top:1px solid var(--cmp-border); }
.composer-footer > button { padding-top:6px; padding-bottom:6px; }
.composer-status { flex:1; min-width:0; overflow:hidden; color:var(--cmp-muted); font-size:12px; text-overflow:ellipsis; white-space:nowrap; }
.composer-error { color:var(--cmp-danger); }
.composer-message { color:var(--ui-success-fg, #24834b); }
.composer-spinner { animation:history-spin 1s linear infinite; }
@media (max-width: 640px) {
  .composer-body { grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr) minmax(0,1fr); }
  .composer-list { border-right:0; border-bottom:1px solid var(--cmp-border); }
}
</style>
