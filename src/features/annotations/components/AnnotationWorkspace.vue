<script setup lang="ts">
import { ref, watch, type StyleValue } from 'vue'
import type { Rect } from '@/shared/geometry/selection'
import type { AnnotationEditor, AnnotationTool } from '@/features/annotations/use-annotations'
import AnnotationSurface from '@/features/annotations/components/AnnotationSurface.vue'
import AnnotationToolbar from '@/features/annotations/components/AnnotationToolbar.vue'

/** 截图和实时录制共用编辑页面，宿主只提供背景、坐标及保存/录制动作。 */
defineProps<{
  editor: AnnotationEditor; width: number; height: number; bounds: Rect
  surfaceVisible: boolean; toolbarVisible: boolean; enabled: boolean; active: boolean; disabled: boolean
  surfaceStyle?: StyleValue; toolbarStyle?: StyleValue; outlined?: boolean; defaultExpanded?: boolean; pointerLabel?: string
}>()
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
  if ((event.target as Element).closest('[aria-haspopup="menu"]')) emit('activate')
}
</script>

<template>
  <div v-if="surfaceVisible" class="annotation-workspace-stage" :class="{ outlined }" :style="surfaceStyle">
    <AnnotationSurface :editor="editor" :width="width" :height="height" :bounds="bounds" :enabled="enabled" />
  </div>
  <div v-if="toolbarVisible" ref="toolbar" class="capture-toolbar annotation-toolbar annotation-workspace-toolbar" :style="toolbarStyle" @pointerdown.capture="activate" @keydown.capture="activate" @pointerdown.stop @contextmenu.stop.prevent>
    <AnnotationToolbar collapsible :default-expanded="defaultExpanded" :pointer-label="pointerLabel" :editor="editor" :disabled="disabled" :active="active" @tool="emit('tool', $event)" @float="emit('float', $event)" @reset-position="emit('resetPosition')">
      <template #leading><slot name="leading" /></template>
      <template #actions><slot name="actions" /></template>
      <template #options><slot name="options" /></template>
    </AnnotationToolbar>
    <slot name="footer" />
  </div>
</template>

<style scoped>
.annotation-workspace-stage { position: absolute; inset: 0; pointer-events: none; }
.annotation-workspace-stage.outlined { outline: 2px solid #3b82f6; }
.capture-toolbar.annotation-workspace-toolbar { position: absolute; z-index: 5; display: flex; flex-direction: column; gap: 4px; width: max-content; max-width: calc(100vw - 16px); padding: 5px; border: 1px solid #d4d4d8; border-radius: 6px; background: #fff; color: #24292f; box-shadow: 0 4px 16px #0003; cursor: default; font: 13px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
.annotation-workspace-toolbar :deep(.annotation-tools) { width: max-content; max-width: 100%; }
.annotation-workspace-toolbar :deep(button) { display: inline-flex; flex: none; align-items: center; justify-content: center; gap: 4px; height: 28px; min-width: 28px; padding: 3px 6px; border: 0; border-radius: 5px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
.annotation-workspace-toolbar :deep(button:hover:enabled) { background: #eff6ff; }
.annotation-workspace-toolbar :deep(button:disabled) { opacity: .45; cursor: default; }
.annotation-workspace-toolbar :deep(button:focus-visible) { outline: 2px solid #2563eb; outline-offset: -2px; }
.annotation-workspace-toolbar :deep(.copy-button) { color: #fff; background: #2563eb; }
:global([data-theme=dark]) .annotation-workspace-toolbar { background: #20252c; color: #e6edf3; border-color: #444c56; }
:global([data-theme=dark]) .annotation-workspace-toolbar :deep(button:hover:enabled) { background: #303f56; }
</style>
