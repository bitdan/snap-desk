<script setup lang="ts">
import type { StyleValue } from 'vue'
import type { Annotation } from '@/features/annotations/annotations'
import type { Rect } from '@/shared/geometry/selection'
import type { AnnotationEditor, AnnotationTool } from '@/features/annotations/use-annotations'
import AnnotationSurface from '@/features/annotations/components/AnnotationSurface.vue'
import AnnotationToolbarHost from '@/features/annotations/components/AnnotationToolbarHost.vue'

/** 截图和实时录制共用编辑页面，宿主只提供背景、坐标及保存/录制动作。 */
defineProps<{
  presentation?: Annotation[]
  editor: AnnotationEditor; width: number; height: number; bounds: Rect
  surfaceVisible: boolean; toolbarVisible: boolean; enabled: boolean; active: boolean; disabled: boolean
  surfaceStyle?: StyleValue; toolbarStyle?: StyleValue; outlined?: boolean; defaultExpanded?: boolean; pointerLabel?: string
}>()
const emit = defineEmits<{
  tool: [tool: AnnotationTool]; float: [event: PointerEvent]; resetPosition: []
  toolbarElement: [element: HTMLElement | null]
  activate: []
}>()
</script>

<template>
  <div v-if="surfaceVisible" class="annotation-workspace-stage" :class="{ outlined }" :style="surfaceStyle">
    <AnnotationSurface :editor="editor" :presentation="presentation" :width="width" :height="height" :bounds="bounds" :enabled="enabled" />
  </div>
  <AnnotationToolbarHost v-if="toolbarVisible" :style="toolbarStyle" :editor="editor" :disabled="disabled" :active="active" :default-expanded="defaultExpanded" :pointer-label="pointerLabel"
    @tool="emit('tool', $event)" @float="emit('float', $event)" @reset-position="emit('resetPosition')" @activate="emit('activate')" @toolbar-element="emit('toolbarElement', $event)">
    <template #leading><slot name="leading" /></template>
    <template #actions><slot name="actions" /></template>
    <template #options><slot name="options" /></template>
    <template #footer><slot name="footer" /></template>
  </AnnotationToolbarHost>
</template>

<style scoped>
.annotation-workspace-stage { position: absolute; inset: 0; pointer-events: none; }
.annotation-workspace-stage.outlined { outline: 2px solid #3b82f6; }
</style>
