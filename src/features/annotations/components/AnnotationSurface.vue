<script setup lang="ts">
import { annotationTextLayout } from '../annotation-text-layout'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
import { t } from '@/shared/i18n/i18n'
import AnnotationStack from '@/features/annotations/components/AnnotationStack'
import { computed, nextTick, ref, useId, watch } from 'vue'
import { shapePath, shapeBounds, hitTest, isRaster, type Annotation } from '@/features/annotations/annotations'
import type { AnnotationEditor } from '@/features/annotations/use-annotations'
import type { Point, Rect, Handle } from '@/shared/geometry/selection'

const props = defineProps<{ editor: AnnotationEditor; presentation?: Annotation[]; preserveAspectRatio?: string; width: number; height: number; bounds: Rect; enabled: boolean }>()
const emit = defineEmits<{ backgroundPointerdown: [event: PointerEvent] }>()
const clipId = `annotation-${useId().replaceAll(':', '')}`
const textInput = ref<HTMLTextAreaElement | null>(null)
const textBox = computed(() => props.editor.textBox())
const textHeight = ref(36)
const composing = ref(false)
const hoveringObject = ref(false)
let pointerOutside = false
useEscapeLayer(() => props.enabled && props.editor.textEditing, () => props.editor.cancelText())
const atTextBoundary = ref(false)
let acceptedText = ''
/** 固定左上角，输入框只利用起点右侧和下方的空间；预览和提交共用换行规则。 */
function measureText(text: string) {
  const draft = props.editor.draft
  if (!draft || typeof document === 'undefined') return null
  const context = document.createElement('canvas').getContext('2d')
  if (!context) return null
  context.font = `${draft.fontSize ?? 24}px ${({ sans: 'sans-serif', serif: 'serif', mono: 'monospace' })[draft.fontFamily ?? 'sans']}`
  const width = props.editor.textFrame?.width ?? props.bounds.x + props.bounds.width - draft.start.x
  const layout = annotationTextLayout(text, draft.fontSize ?? 24, width, value => context.measureText(value).width)
  return { ...layout, fitsWidth: layout.lines.every(line => context.measureText(line).width + 4 <= width) }
}
watch(() => [props.editor.textEditing, props.editor.draft?.text, props.editor.draft?.fontSize, props.editor.draft?.fontFamily], () => {
  if (!props.editor.textEditing || props.editor.textFrame) return
  const layout = measureText(props.editor.draft?.text ?? '')
  if (!layout) return
  props.editor.textMeasuredWidth = Math.max(120, layout.width)
  props.editor.textMeasuredHeight = layout.height
}, { flush: 'post' })
/** 空间耗尽时保留最后一份可显示的内容；IME 只在组词结束后校验，不截断候选词。 */
function acceptInput(value: string, input?: HTMLTextAreaElement, deleting = false) {
  const layout = measureText(value)
  if (layout && (!layout.fitsWidth || layout.height > textBox.value.height) && !deleting) {
    atTextBoundary.value = true
    if (props.editor.draft) props.editor.draft.text = acceptedText
    if (input) input.value = acceptedText
    return
  }
  acceptedText = value; atTextBoundary.value = false
  if (props.editor.draft) props.editor.draft.text = value
}
function inputText(event: InputEvent) {
  if (event.isComposing || composing.value) return
  const input = event.target as HTMLTextAreaElement
  acceptInput(input.value, input, event.inputType?.startsWith('delete') ?? false)
}
// 初始只显示一行；用浏览器实际换行高度扩展，达到可用高度后在框内滚动。
watch(() => [props.editor.textEditing, props.editor.textAutoHeight, props.editor.draft?.text, props.editor.draft?.fontSize, textBox.value.width, textBox.value.height], async () => {
  await nextTick()
  const input = textInput.value
  if (!input) return
  input.style.height = '0px'
  // 新建和二次编辑均贴合内容，仅在用户手动拖拽后使用指定高度。
  textHeight.value = props.editor.textAutoHeight ? Math.min(textBox.value.height, Math.max(Math.ceil((props.editor.draft?.fontSize ?? 24) * 1.3 + 4), input.scrollHeight, props.editor.textMeasuredHeight)) : textBox.value.height
  input.style.height = '100%'
}, { flush: 'post' })
watch(() => props.editor.textEditing, async active => {
  pointerOutside = false; composing.value = false; atTextBoundary.value = false
  acceptedText = props.editor.draft?.text ?? ''
  if (active) { await nextTick(); textInput.value?.focus() }
})
async function finishText() {
  await props.editor.finishText()
  // 原生保存或文字排版失败时保留原输入，焦点回到框内以便继续修改。
  if (props.editor.textEditing) { await nextTick(); textInput.value?.focus() }
}
/** 移出即完成，但拖拽尺寸、选字和输入法组词期间不提交半成品。 */
function leaveText(event: PointerEvent) {
  pointerOutside = true
  if (!event.buttons && !composing.value && !props.editor.pending) void finishText()
}
function compositionEnd(event?: CompositionEvent) {
  composing.value = false
  const input = event?.target as HTMLTextAreaElement | undefined
  acceptInput(input?.value ?? props.editor.draft?.text ?? '', input)
  if (pointerOutside) void nextTick(() => finishText())
}
function doubleClick(event: MouseEvent) {
  if (!props.enabled || props.editor.pending || props.editor.textEditing) return
  const svg = event.currentTarget as SVGSVGElement
  const matrix = svg.getScreenCTM?.()
  const scale = matrix ? Math.hypot(matrix.a, matrix.b) : svg.getBoundingClientRect().width / props.width
  const index = hitTest(props.editor.history.present, point(event), 6 / Math.max(Number.EPSILON, scale), true, true)
  if (index >= 0) props.editor.editText(index)
}
function textKey(event: KeyboardEvent) {
  // Esc 只取消当前文字编辑；失败草稿也必须能够退出，不再重复尝试保存。
  event.stopPropagation()
  if (event.isComposing || event.keyCode === 229) return
  if (event.key === 'Escape') { event.preventDefault(); if (!event.repeat) props.editor.cancelText() }
  else if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); if (!event.repeat) void finishText() }
}
const shapes = computed(() => {
  const e = props.editor
  if (!e.draft) return props.presentation ?? e.history.present
  const present = props.presentation ?? e.history.present
  return e.selected >= 0 ? present.map((s, i) => i === e.selected ? e.draft! : s) : [...present, ...(e.arrowDraft ? [e.arrowDraft] : []), e.draft]
})
// 擦除仅作用于之前的图层，之后新增的标注仍可覆盖擦除区域。
const erasures = computed(() => shapes.value.map((shape, index) => ({ shape, index })).filter(item => item.shape.kind === 'erase'))
const chosen = computed(() => props.editor.selected >= 0 ? shapes.value[props.editor.selected] : null)
const spotlights = computed(() => shapes.value.filter(s => s.kind === 'spotlight' && (s.opacity ?? 1) > 0))
const spotlightIndex = computed(() => shapes.value.findIndex(s => s.kind === 'spotlight' && (s.opacity ?? 1) > 0))
const spotlightMaskId = `${clipId}-spotlight`
const handles = computed(() => {
  const s = chosen.value
  if (!s) return []
  if (s.kind === 'line' || s.kind === 'arrow') return [{ name: 'start', ...s.start }, { name: 'end', ...s.end }]
  const b = shapeBounds(s)
  return [ ['nw', 0, 0], ['n', .5, 0], ['ne', 1, 0], ['e', 1, .5], ['se', 1, 1], ['s', .5, 1], ['sw', 0, 1], ['w', 0, .5] ].map(([name, x, y]) => ({ name: String(name), x: b.x + Number(x) * b.width, y: b.y + Number(y) * b.height }))
})
function point(event: MouseEvent): Point {
  const svg = event.currentTarget as SVGSVGElement
  // 录后预览使用 meet 保留视频比例；逆矩阵同时扣除黑边和缩放，不能按整个容器拉伸坐标。
  const matrix = svg.getScreenCTM?.()
  if (matrix) {
    const point = svg.createSVGPoint(); point.x = event.clientX; point.y = event.clientY
    const local = point.matrixTransform(matrix.inverse())
    return { x: local.x, y: local.y }
  }
  const box = svg.getBoundingClientRect()
  return { x: (event.clientX - box.left) * props.width / box.width, y: (event.clientY - box.top) * props.height / box.height }
}
function down(event: PointerEvent) {
  if (!props.enabled || event.button !== 0) return
  // 阻止 SVG 默认鼠标行为夺走新建输入框的焦点；输入框内部仍保留原生选字行为。
  event.preventDefault()
  const resizeHandle = (event.target as Element).getAttribute('data-text-resize') as Handle | null
  if (props.editor.textEditing && resizeHandle) {
    props.editor.beginTextResize(point(event), resizeHandle, textHeight.value)
    ;(event.currentTarget as SVGSVGElement).setPointerCapture(event.pointerId)
    return
  }
  if (props.editor.textEditing) { if (!composing.value) void finishText(); return }
  const p = point(event), b = props.bounds
  if (p.x < b.x || p.y < b.y || p.x > b.x + b.width || p.y > b.y + b.height) return
  const handle = (event.target as Element).getAttribute('data-handle') as Handle | 'start' | 'end' | null
  const svg = event.currentTarget as SVGSVGElement
  const matrix = svg.getScreenCTM?.()
  const scale = matrix ? Math.hypot(matrix.a, matrix.b) : svg.getBoundingClientRect().width / props.width
  // 指针工具命中空白时把拖动交给宿主调整裁剪；容差与对象选择保持相同的屏幕像素。
  if (!handle && props.editor.tool === 'edit' && hitTest(props.editor.history.present, p, 6 / Math.max(Number.EPSILON, scale), true, true) < 0) {
    emit('backgroundPointerdown', event); return
  }
  props.editor.begin(p, 6 / Math.max(Number.EPSILON, scale), handle ?? undefined, event.altKey)
  if (props.editor.interacting && !props.editor.textEditing) svg.setPointerCapture(event.pointerId)
}
function move(event: PointerEvent) {
  const p = point(event)
  props.editor.drag(p)
  const svg = event.currentTarget as SVGSVGElement, matrix = svg.getScreenCTM?.()
  const scale = matrix ? Math.hypot(matrix.a, matrix.b) : svg.getBoundingClientRect().width / props.width
  hoveringObject.value = !event.altKey && !props.editor.interacting && !['pen', 'highlighter', 'erase', 'polyline', 'number', 'spotlight'].includes(props.editor.tool) && hitTest(props.editor.history.present, p, 6 / Math.max(Number.EPSILON, scale), true, true) >= 0
}
async function up() {
  await props.editor.finish()
  if (props.editor.textEditing) { await nextTick(); textInput.value?.focus() }
}
</script>

<template>
  <svg class="annotation-surface" :class="{ enabled, 'object-hover': hoveringObject }" :viewBox="`0 0 ${width} ${height}`" :preserveAspectRatio="preserveAspectRatio ?? 'none'"
    @pointerdown.stop="down" @pointermove.stop="move" @pointerleave="hoveringObject = false" @pointerup.stop="up" @dblclick.stop.prevent="doubleClick" @pointercancel.stop="!editor.pending && editor.abort()" @contextmenu.stop.prevent="!editor.pending && editor.abort()">
    <defs>
      <clipPath :id="clipId"><rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" /></clipPath>
      <!-- 黑色亮区在同一遮罩内取并集，重叠区域不会像 evenodd 那样重新变暗。 -->
      <mask :id="spotlightMaskId" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" style="mask-type: luminance">
        <rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" fill="white" />
        <rect v-for="(s, index) in spotlights" :key="index" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" fill="black" />
      </mask>
      <mask v-for="item in erasures" :id="`${clipId}-erase-${item.index}`" :key="item.index" maskUnits="userSpaceOnUse" :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" style="mask-type: luminance">
        <rect :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" fill="white" />
        <path :d="shapePath(item.shape)" :opacity="item.shape.opacity ?? 1" stroke="black" :stroke-width="item.shape.width" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      </mask>
    </defs>
    <g :clip-path="`url(#${clipId})`">
      <AnnotationStack :shapes="shapes" :mask-prefix="clipId" v-slot="{ s, index }">
        <template v-if="editor.textEditing && s === editor.draft" />
        <template v-else-if="s.kind === 'spotlight'">
          <rect v-if="index === spotlightIndex" :x="bounds.x" :y="bounds.y" :width="bounds.width" :height="bounds.height" :mask="`url(#${spotlightMaskId})`" :fill="s.color" :opacity="s.opacity ?? 1" />
        </template>
        <image v-else-if="isRaster(s) && s.raster" :href="`data:image/png;base64,${s.raster}`" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" preserveAspectRatio="none" :opacity="s.opacity ?? 1" />
        <rect v-else-if="isRaster(s)" :opacity="s.opacity ?? 1" :x="shapeBounds(s).x" :y="shapeBounds(s).y" :width="shapeBounds(s).width" :height="shapeBounds(s).height" fill="#94a3b844" stroke="#8b5cf6" vector-effect="non-scaling-stroke" />
        <path v-else-if="s.kind !== 'erase'" :d="shapePath(s)" :stroke="s.color" :stroke-width="s.width" :opacity="s.opacity ?? 1" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      </AnnotationStack>
    </g>
    <foreignObject v-if="enabled && editor.textEditing && editor.draft" :x="textBox.x" :y="textBox.y" :width="textBox.width" :height="textHeight"
      @pointerdown.stop @pointermove.stop @pointerup.stop @pointercancel.stop @contextmenu.stop @pointerenter="pointerOutside = false" @pointerleave="leaveText" @keydown="textKey">
      <textarea ref="textInput" v-model="editor.draft.text" class="inline-text" :class="{ 'arrow-text': editor.arrowDraft }" :aria-label='t("标注文字")' :placeholder='t("输入文字")' maxlength="500" :readonly="editor.pending" @input="inputText" @compositionstart="composing = true" @compositionend="compositionEnd"
        :title="atTextBoundary ? t('已到选区边界，可缩小字号继续输入') : t('Enter 完成，Esc 取消，Shift+Enter 换行')" :style="{ color: editor.draft.color, fontSize: `${editor.draft.fontSize ?? 24}px`, fontFamily: ({ sans: 'sans-serif', serif: 'serif', mono: 'monospace' })[editor.draft.fontFamily ?? 'sans'] }" />
    </foreignObject>
    <!-- 输入框外的三个手柄分别调整宽度、高度和两者，不锁定长宽比。 -->
    <g v-if="enabled && editor.textEditing && !editor.pending" class="annotation-selection">
      <path data-text-resize="e" :d="`M ${textBox.x + textBox.width} ${textBox.y + textHeight / 2} h .001`" class="text-resize horizontal" />
      <path data-text-resize="s" :d="`M ${textBox.x + textBox.width / 2} ${textBox.y + textHeight} h .001`" class="text-resize vertical" />
      <path data-text-resize="se" :d="`M ${textBox.x + textBox.width} ${textBox.y + textHeight} h .001`" class="text-resize" />
    </g>
    <g v-if="enabled && chosen && !editor.textEditing" class="annotation-selection">
      <rect :x="shapeBounds(chosen).x" :y="shapeBounds(chosen).y" :width="Math.max(1, shapeBounds(chosen).width)" :height="Math.max(1, shapeBounds(chosen).height)" fill="none" stroke="#8b5cf6" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" />
      <path v-for="h in handles" :key="h.name" :data-handle="h.name" :d="`M ${h.x} ${h.y} h .001`" stroke="white" stroke-width="9" stroke-linecap="square" vector-effect="non-scaling-stroke" class="edit-handle" />
    </g>
  </svg>
</template>

<style scoped>
.annotation-surface { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.annotation-surface.enabled { pointer-events: auto; cursor: crosshair; touch-action: none; }
.annotation-surface.enabled.object-hover { cursor: move; }
.annotation-selection { pointer-events: none; }
.edit-handle { pointer-events: stroke; cursor: nwse-resize; filter: drop-shadow(0 0 1px #7c3aed); }
.text-resize { pointer-events: stroke; cursor: nwse-resize; stroke: #8b5cf6; stroke-width: 10; stroke-linecap: square; vector-effect: non-scaling-stroke; }
.text-resize.horizontal { cursor: ew-resize; }
.text-resize.vertical { cursor: ns-resize; }
.inline-text { display: block; box-sizing: border-box; width: 100%; height: 100%; margin: 0; padding: 2px; border: 0; outline: 1px dashed #8b5cf6; outline-offset: -1px; border-radius: 0; background: #ffffffdd; line-height: 1.3; resize: none; cursor: text; white-space: pre-wrap; overflow-wrap: anywhere; overflow-y: auto; }
.inline-text.arrow-text { text-align: center; }
</style>
