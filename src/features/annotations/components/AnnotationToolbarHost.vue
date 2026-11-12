<script setup lang="ts">
import { ref, watch } from 'vue'
import type { AnnotationToolbarEditor, AnnotationTool } from '@/features/annotations/use-annotations'
import AnnotationToolbar from './AnnotationToolbar.vue'

/** 统一截图浮层和录屏控制窗的工具栏呈现；嵌入模式仅改变定位，不改变工具和样式。 */
withDefaults(defineProps<{
  editor: AnnotationToolbarEditor; disabled: boolean; active: boolean
  placement?: 'floating' | 'embedded'; defaultExpanded?: boolean; pointerLabel?: string; movable?: boolean
}>(), { placement: 'floating' })
const emit = defineEmits<{
  tool: [tool: AnnotationTool]; float: [event: PointerEvent]; resetPosition: []
  toolbarElement: [element: HTMLElement | null]
  activate: []
}>()
const toolbar = ref<HTMLElement | null>(null)
watch(toolbar, element => emit('toolbarElement', element), { flush: 'post' })
function activate(event: Event) {
  // 折叠工具的菜单通过 Portal 绘制在工具栏外，打开前恢复完整输入区域。
  // 普通工具点击由 tool 事件切换，不能在捕获阶段提前激活后又被当作重复点击关闭。
  if (event.type === 'keydown' && !['Enter', ' ', 'ArrowDown', 'ArrowUp'].includes((event as KeyboardEvent).key)) return
  if ((event.target as Element).closest('[aria-haspopup="menu"], [aria-haspopup="listbox"]')) emit('activate')
}
</script>

<template>
  <div ref="toolbar" class="capture-toolbar annotation-toolbar annotation-workspace-toolbar" :class="placement" @pointerdown.capture="activate" @keydown.capture="activate" @pointerdown.stop @contextmenu.stop.prevent>
    <AnnotationToolbar :movable="movable" collapsible :default-expanded="defaultExpanded" :pointer-label="pointerLabel" :editor="editor" :disabled="disabled" :active="active" @tool="emit('tool', $event)" @float="emit('float', $event)" @reset-position="emit('resetPosition')">
      <template #leading><slot name="leading" /></template>
      <template v-if="$slots.actions" #actions><slot name="actions" /></template>
      <template #options><slot name="options" /></template>
    </AnnotationToolbar>
    <slot name="footer" />
  </div>
</template>

<style scoped>
/* 宿主只负责定位；外观由 AnnotationToolbar 的胶囊行和 footer 自带的胶囊提供。 */
.capture-toolbar.annotation-workspace-toolbar { position: absolute; z-index: 5; display: flex; flex-direction: column; align-items: flex-start; gap: 6px; width: max-content; max-width: calc(100vw - 16px); color: var(--tb-text); cursor: default; font: 13px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
.capture-toolbar.annotation-workspace-toolbar.embedded { position: relative; width: 100%; max-width: 100%; box-sizing: border-box; }
.annotation-workspace-toolbar :deep(.annotation-tools) { width: max-content; max-width: 100%; }
.annotation-workspace-toolbar :deep(button) { display: inline-flex; flex: none; align-items: center; justify-content: center; gap: 4px; height: 30px; min-width: 30px; padding: 0 6px; border: 0; border-radius: 6px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
.annotation-workspace-toolbar :deep(button:hover:enabled) { background: var(--tb-hover); }
.annotation-workspace-toolbar :deep(button[aria-pressed=true]) { color: var(--tb-active-fg); background: var(--tb-active-bg); }
.annotation-workspace-toolbar :deep(button:disabled) { opacity: .4; cursor: default; }
.annotation-workspace-toolbar :deep(button:focus-visible) { outline: 2px solid #2563eb; outline-offset: -2px; }
/* 一条工具栏只有一个实色主按钮。 */
.annotation-workspace-toolbar :deep(.copy-button) { margin-left: 4px; color: #fff; background: #2563eb; }
.annotation-workspace-toolbar :deep(.copy-button:hover:enabled) { background: #1d4ed8; }
</style>
