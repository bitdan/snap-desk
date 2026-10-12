<script setup lang="ts">
import { computed } from 'vue'
import { Eraser, Undo2, Redo2, Trash2, GripVertical } from 'lucide-vue-next'
import { annotationTools as tools } from '@/lib/annotation-tools'
import type { AnnotationEditor, AnnotationTool } from '@/lib/use-annotations'
const props = defineProps<{ editor: AnnotationEditor; disabled: boolean; active: boolean }>()
const emit = defineEmits<{ tool: [tool: AnnotationTool] }>()

const locked = computed(() => props.disabled || props.editor.pending || props.editor.interacting)
const textMode = computed(() => props.editor.tool === 'text' || props.editor.history.present[props.editor.selected]?.kind === 'text')
const filterMode = computed(() => ['mosaic', 'blur'].includes(props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool))
const numberMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'number')
const spotlightMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'spotlight')
const magnifyMode = computed(() => (props.editor.history.present[props.editor.selected]?.kind ?? props.editor.tool) === 'magnify')
function value(event: Event) { return (event.target as HTMLInputElement).value }
</script>

<template>
  <div class="annotation-tools" @pointerdown.stop @contextmenu.stop.prevent>
    <div class="annotation-tool-row" role="toolbar" aria-label="标注工具">
      <GripVertical class="tool-grip" :size="16" aria-hidden="true" />
      <slot name="leading" />
      <button v-for="item in tools" :key="item.tool" :disabled="locked" :title="item.name" :aria-label="item.name" :aria-pressed="active && editor.tool === item.tool" @click="emit('tool', item.tool)"><component :is="item.icon" :size="18" /></button>
      <span class="tool-divider" />
      <button :disabled="locked || !editor.history.past.length" title="撤销 Ctrl/⌘+Z" aria-label="撤销" @click="editor.travel('undo')"><Undo2 :size="16" /></button>
      <button :disabled="locked || !editor.history.future.length" title="重做 Ctrl/⌘+Shift+Z" aria-label="重做" @click="editor.travel('redo')"><Redo2 :size="16" /></button>
      <button :disabled="locked || editor.selected < 0" title="删除选中标注 Delete" aria-label="删除选中标注" @click="editor.remove"><Trash2 :size="16" /></button>
      <button :disabled="locked || !editor.history.present.length" title="清空标注，可撤销恢复" aria-label="清空标注" @click="editor.clear"><Eraser :size="16" /><span class="clear-label">全</span></button>
      <span class="tool-divider" />
      <slot name="actions" />
    </div>
    <div v-if="active && (editor.tool !== 'edit' || editor.selected >= 0)" class="annotation-tool-row annotation-options" aria-label="标注样式">
      <label>颜色 <input type="color" :value="editor.color" :disabled="locked || filterMode" @change="editor.style({ color: value($event) })" /></label>
      <label v-if="!numberMode && !spotlightMode">{{ magnifyMode ? '边框' : '粗细' }} <input type="number" min="1" max="64" :value="editor.width" :disabled="locked" @change="editor.style({ width: Math.min(64, Math.max(1, Number(value($event)) || 3)) })" /></label>
      <label v-if="magnifyMode">倍率 <select :value="editor.magnification" :disabled="locked" @change="editor.style({ magnification: Number(value($event)) })"><option :value="2">2×</option><option :value="3">3×</option><option :value="4">4×</option></select></label>
      <label>不透明度 <select :value="editor.opacity" :disabled="locked || filterMode" @change="editor.style({ opacity: Number(value($event)) })"><option :value="1">100%</option><option :value="0.7">70%</option><option :value="0.35">35%</option><option :value="0.15">15%</option></select></label>
      <template v-if="textMode">
      <input v-if="editor.selected >= 0" class="annotation-text" aria-label="修改标注文字" placeholder="修改选中文字" maxlength="500" :value="editor.text" :disabled="locked" @change="editor.style({ text: value($event) })" />
      <span v-else class="annotation-hint">点击图片输入文字；Enter 完成，Shift+Enter 换行，Esc 取消</span>
      <label>字号 <input type="number" min="8" max="128" :value="editor.fontSize" :disabled="locked" @change="editor.style({ fontSize: Math.min(128, Math.max(8, Number(value($event)) || 24)) })" /></label>
      <select aria-label="字体" :value="editor.fontFamily" :disabled="locked" @change="editor.style({ fontFamily: value($event) as 'sans' | 'serif' | 'mono' })"><option value="sans">无衬线</option><option value="serif">衬线</option><option value="mono">等宽</option></select>
      </template>
      <span v-else class="annotation-hint">{{ spotlightMode ? '拖框保留亮区，框外覆盖遮罩' : magnifyMode ? '拖框放大中心底图；移动后重新取样' : numberMode ? '点击连续放置序号；选择 / 编辑可移动和缩放' : editor.tool === 'polyline' ? '逐点点击，Enter 完成' : '选择箭头指针可二次编辑' }}</span>
    </div>
    <slot v-else name="options" />
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
.tool-grip { color: #d4d4d8; }
.tool-divider { width: 1px; height: 17px; background: #e4e4e7; margin: 0 4px; }
.clear-label { font-size: 8px; position: absolute; right: 1px; bottom: 0; }
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
