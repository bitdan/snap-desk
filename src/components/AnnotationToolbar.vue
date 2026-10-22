<script setup lang="ts">
import { computed, ref, nextTick, onUnmounted } from 'vue'
import { Eraser, Undo2, Redo2, Trash2, GripVertical, RotateCcw } from 'lucide-vue-next'
import { annotationTools } from '@/lib/annotation-tools'
import { readToolOrder, moveTool, normalizeToolOrder, toolbarOrderKey } from '@/lib/toolbar-order'
import type { AnnotationEditor, AnnotationTool } from '@/lib/use-annotations'
const props = defineProps<{ editor: AnnotationEditor; disabled: boolean; active: boolean }>()
const emit = defineEmits<{ tool: [tool: AnnotationTool]; float: [event: PointerEvent]; resetPosition: [] }>()
// 顺序在截图和贴图窗口之间共享；存储失败不阻断当前窗口的编辑。
const order = ref(readToolOrder())
const orderError = ref('')
const tools = computed(() => order.value.map(tool => annotationTools.find(item => item.tool === tool)!))
// 指针捕获后 hover 仍停在源按钮，单独记录落点才能准确提示将要移动到的位置。
const sortingTool = ref<AnnotationTool | null>(null)
const sortingTarget = ref<AnnotationTool | null>(null)
let dragged: { tool: AnnotationTool; id: number; x: number; y: number; moved: boolean; handle: HTMLElement } | null = null
let suppressClick = false
/** 使用指针捕获排序，避免桌面 WebView 的原生文件拖放拦截 HTML 拖放。 */
function startOrder(event: PointerEvent, tool: AnnotationTool) {
  if (locked.value || event.button !== 0 || dragged) return
  suppressClick = false
  dragged = { tool, id: event.pointerId, x: event.clientX, y: event.clientY, moved: false, handle: event.currentTarget as HTMLElement }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}
function moveOrder(event: PointerEvent) {
  if (!dragged || dragged.id !== event.pointerId) return
  if (Math.hypot(event.clientX - dragged.x, event.clientY - dragged.y) > 5) dragged.moved = true
  if (dragged.moved) {
    sortingTool.value = dragged.tool
    sortingTarget.value = orderTarget(event)?.dataset.annotationTool as AnnotationTool | undefined ?? null
  }
}
/** 只接受同一工具行里的按钮，拖到图片或另一个工具栏时不改顺序。 */
function orderTarget(event: PointerEvent) {
  const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-annotation-tool]')
  return target && dragged?.handle.parentElement === target.parentElement ? target : null
}
function finishOrder(event: PointerEvent) {
  const source = dragged
  if (!source || source.id !== event.pointerId) return
  // 抬起时也检查位移，避免系统合并 pointermove 后把拖动当作点击。
  moveOrder(event)
  const target = orderTarget(event)
  dragged = null
  sortingTool.value = null
  sortingTarget.value = null
  suppressClick = source.moved
  const button = event.currentTarget as HTMLElement
  if (button.hasPointerCapture(event.pointerId)) button.releasePointerCapture(event.pointerId)
  if (source.moved && !locked.value && target && button.parentElement === target.parentElement) {
    saveOrder(moveTool(order.value, source.tool, target.dataset.annotationTool as AnnotationTool))
  }
}
function choose(event: MouseEvent, tool: AnnotationTool) {
  // 只吞掉拖动产生的鼠标点击；取消拖动后仍允许 Enter/空格选工具。
  if (suppressClick && event.detail !== 0) { suppressClick = false; return }
  suppressClick = false
  emit('tool', tool)
}
/** 系统取消或窗口失焦时释放指针，避免下一次点击误用旧的排序起点。 */
function cancelOrder(event?: PointerEvent) {
  if (!dragged || (event && event.pointerId !== dragged.id)) return
  const source = dragged
  dragged = null
  sortingTool.value = null
  sortingTarget.value = null
  suppressClick = true
  if (source.handle.hasPointerCapture(source.id)) source.handle.releasePointerCapture(source.id)
}
function blurOrder() { cancelOrder() }
function saveOrder(next: AnnotationTool[]) {
  order.value = next
  try { localStorage.setItem(toolbarOrderKey, JSON.stringify(next)); orderError.value = '' }
  catch { orderError.value = '工具顺序仅在当前窗口生效，无法保存。' }
}
function orderKey(event: KeyboardEvent, tool: AnnotationTool) {
  // Escape 只取消当前排序，不把事件传给截图窗口的退出快捷键。
  if (event.key === 'Escape' && dragged) {
    event.preventDefault(); event.stopPropagation(); cancelOrder(); return
  }
  if (!event.altKey || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return
  event.preventDefault(); event.stopPropagation()
  if (locked.value) return
  const target = order.value[order.value.indexOf(tool) + (event.key === 'ArrowLeft' ? -1 : 1)]
  if (target) {
    const button = event.currentTarget as HTMLElement
    saveOrder(moveTool(order.value, tool, target))
    // DOM 重排可能让浏览器丢失焦点，恢复到原工具以支持连续键盘排序。
    void nextTick(() => button.focus({ preventScroll: true }))
  }
}
function syncOrder(event: StorageEvent) {
  if (event.key === toolbarOrderKey || event.key === null) order.value = readToolOrder()
}
window.addEventListener('storage', syncOrder)
window.addEventListener('blur', blurOrder)
onUnmounted(() => {
  cancelOrder()
  window.removeEventListener('storage', syncOrder)
  window.removeEventListener('blur', blurOrder)
})

const locked = computed(() => props.disabled || props.editor.pending || props.editor.interacting)
const textMode = computed(() => props.editor.tool === 'text' || props.editor.history.present[props.editor.selected]?.kind === 'text')
const filterMode = computed(() => ['mosaic', 'blur', 'erase'].includes(props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool))
const numberMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'number')
const spotlightMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'spotlight')
const magnifyMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'magnify')
function value(event: Event) { return (event.target as HTMLInputElement).value }
</script>

<template>
  <div class="annotation-tools" @pointerdown.stop @contextmenu.stop.prevent>
    <div class="annotation-tool-row" role="toolbar" aria-label="标注工具">
      <button class="tool-grip" :disabled="locked" title="拖动工具栏；双击恢复位置" aria-label="拖动工具栏，双击或 Enter 恢复位置" @pointerdown.stop="emit('float', $event)" @dblclick="emit('resetPosition')" @keydown.enter.stop.prevent="emit('resetPosition')"><GripVertical :size="16" /></button>
      <slot name="leading" />
      <button v-for="item in tools" :key="item.tool" :disabled="locked" :class="{ 'sorting-source': sortingTool === item.tool, 'sorting-target': sortingTarget === item.tool && sortingTarget !== sortingTool }" :title="`${item.name}（拖动排序，Alt+←/→调整顺序）`" :data-annotation-tool="item.tool" @pointerdown="startOrder($event, item.tool)" @pointermove="moveOrder" @pointerup="finishOrder" @pointercancel="cancelOrder" @lostpointercapture="cancelOrder" @dragstart.prevent @keydown="orderKey($event, item.tool)" :aria-label="item.name" :aria-pressed="active && editor.tool === item.tool" @click="choose($event, item.tool)"><component :is="item.icon" :size="18" /></button>
      <button :disabled="locked" title="恢复默认工具顺序" aria-label="恢复默认工具顺序" @click="saveOrder(normalizeToolOrder(null))"><RotateCcw :size="16" /></button>
      <span class="tool-divider" />
      <button :disabled="locked || !editor.history.past.length" title="撤销 Ctrl/⌘+Z" aria-label="撤销" @click="editor.travel('undo')"><Undo2 :size="16" /></button>
      <button :disabled="locked || !editor.history.future.length" title="重做 Ctrl/⌘+Shift+Z" aria-label="重做" @click="editor.travel('redo')"><Redo2 :size="16" /></button>
      <button :disabled="locked || editor.selected < 0" title="删除选中标注 Delete" aria-label="删除选中标注" @click="editor.remove"><Trash2 :size="16" /></button>
      <button :disabled="locked || !editor.history.present.length" title="清空标注，可撤销恢复" aria-label="清空标注" @click="editor.clear"><Eraser :size="16" /><span>清空</span></button>
      <span class="tool-divider" />
      <slot name="actions" />
    </div>
    <div v-if="active && (editor.tool !== 'edit' || editor.selected >= 0)" class="annotation-tool-row annotation-options" aria-label="标注样式">
      <label>颜色 <input type="color" :value="editor.color" :disabled="locked || filterMode" @change="editor.style({ color: value($event) })" /></label>
      <label v-if="!numberMode && !spotlightMode">{{ magnifyMode ? '边框' : editor.tool === 'erase' ? '直径' : '粗细' }} <input type="number" min="1" max="64" :value="editor.width" :disabled="locked" @change="editor.style({ width: Math.min(64, Math.max(1, Number(value($event)) || 3)) })" /></label>
      <label v-if="magnifyMode">倍率 <select :value="editor.magnification" :disabled="locked" @change="editor.style({ magnification: Number(value($event)) })"><option :value="2">2×</option><option :value="3">3×</option><option :value="4">4×</option></select></label>
      <label>不透明度 <select :value="editor.opacity" :disabled="locked || filterMode" @change="editor.style({ opacity: Number(value($event)) })"><option :value="1">100%</option><option :value="0.7">70%</option><option :value="0.35">35%</option><option :value="0.15">15%</option></select></label>
      <template v-if="textMode">
      <input v-if="editor.selected >= 0" class="annotation-text" aria-label="修改标注文字" placeholder="修改选中文字" maxlength="500" :value="editor.text" :disabled="locked" @change="editor.style({ text: value($event) })" />
      <span v-else class="annotation-hint">点击图片输入文字；Enter 完成，Shift+Enter 换行，Esc 取消</span>
      <label>字号 <input type="number" min="8" max="128" :value="editor.fontSize" :disabled="locked" @change="editor.style({ fontSize: Math.min(128, Math.max(8, Number(value($event)) || 24)) })" /></label>
      <select aria-label="字体" :value="editor.fontFamily" :disabled="locked" @change="editor.style({ fontFamily: value($event) as 'sans' | 'serif' | 'mono' })"><option value="sans">无衬线</option><option value="serif">衬线</option><option value="mono">等宽</option></select>
      </template>
      <span v-else class="annotation-hint">{{ editor.tool === 'erase' ? '拖动擦除已有标注并露出底图；可撤销' : spotlightMode ? '拖框保留亮区，框外覆盖遮罩' : magnifyMode ? '拖框放大中心底图；移动后重新取样' : numberMode ? '点击连续放置序号；选择 / 编辑可移动和缩放' : editor.tool === 'polyline' ? '逐点点击，Enter 完成' : '选择箭头指针可二次编辑' }}</span>
    </div>
    <slot v-else name="options" />
    <p v-if="orderError" class="annotation-error" role="status">{{ orderError }}</p>
    <p v-if="editor.error" class="annotation-error" role="alert" :title="editor.error">{{ editor.error }}</p>
  </div>
</template>

<style scoped>
.annotation-tools { min-width: 0; max-width: 100%; font-size: 11px; color: inherit; }
.annotation-tool-row { display: flex; align-items: center; gap: 3px; height: 34px; white-space: nowrap; overflow-x: auto; overflow-y: hidden; scrollbar-width: thin; }
.annotation-tool-row > * { flex-shrink: 0; }
.annotation-tools button, .annotation-tools select { display: inline-flex; align-items: center; justify-content: center; height: 28px; padding: 3px 6px; border-radius: 5px; color: inherit; background: transparent; }
.annotation-tools button { min-width: 28px; position: relative; }
.annotation-tools button:hover:not(:disabled) { background: #f3f0ff; }
.annotation-tools button[aria-pressed=true] { color: #7c3aed; background: #ede9fe; }
.annotation-tools :focus-visible { outline: 2px solid #a78bfa; outline-offset: -2px; }
[data-annotation-tool] { touch-action: none; user-select: none; }
.sorting-source { opacity: .55; }
.sorting-target { outline: 2px solid #2563eb; outline-offset: -2px; }
.tool-grip { color: #71717a; cursor: grab; touch-action: none; }
.tool-grip:active { cursor: grabbing; }
.tool-divider { width: 1px; height: 17px; background: #e4e4e7; margin: 0 4px; }
.annotation-options { gap: 10px; border-top: 1px solid #f4f4f5; }
.annotation-tools label { display: inline-flex; align-items: center; gap: 3px; }
.annotation-tools input { color: #374151; background: white; border: 1px solid #d4d4d8; border-radius: 3px; height: 24px; }
.annotation-tools input[type=color] { width: 28px; padding: 1px; }
.annotation-tools input[type=number] { width: 44px; }
.annotation-text { width: 190px; padding: 2px 6px; }
.annotation-tools button:disabled, .annotation-tools input:disabled, .annotation-tools select:disabled { opacity: .45; }
.annotation-hint { font-size: 10px; color: #71717a; }
.annotation-error { color: #e11d48; line-height: 16px; margin: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
