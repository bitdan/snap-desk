<script setup lang="ts">
import { t } from '@/shared/i18n/i18n'
import { computed, ref, watch } from 'vue'
import { ChevronDown } from 'lucide-vue-next'
import { DropdownMenuRoot, DropdownMenuTrigger, DropdownMenuPortal, DropdownMenuContent, DropdownMenuItem } from 'reka-ui'
import { annotationTools } from '@/features/annotations/annotation-tools'
import type { AnnotationTool } from '@/features/annotations/use-annotations'

const props = defineProps<{ name: string; tools: AnnotationTool[]; selected: AnnotationTool; active: boolean; disabled: boolean; label?: string }>()
const emit = defineEmits<{ select: [tool: AnnotationTool] }>()
const open = ref(false)
const recent = ref(props.tools[0]!)
// 每组记住最后使用的工具；二次点击主按钮可以直接继续使用。
watch(() => props.selected, tool => { if (props.tools.includes(tool)) recent.value = tool }, { immediate: true })
watch(() => props.disabled, disabled => { if (disabled) open.value = false })
const current = computed(() => annotationTools.find(item => item.tool === recent.value)!)
const items = computed(() => props.tools.map(tool => annotationTools.find(item => item.tool === tool)!))
</script>

<template>
  <div class="tool-group">
    <button :disabled="disabled" :title="t(label ?? current.name)" :aria-label="t(label ?? current.name)" :aria-pressed="active && tools.includes(selected)" @click="emit('select', recent)"><component :is="current.icon" :size="18" /></button>
    <DropdownMenuRoot v-if="tools.length > 1" v-model:open="open">
      <DropdownMenuTrigger :disabled="disabled" class="group-trigger" :title="t(name)" :aria-label="t('选择{0}工具', { 0: t(name) })"><ChevronDown :size="12" /></DropdownMenuTrigger>
      <DropdownMenuPortal>
        <!-- Portal 避开工具行的滚动裁剪；菜单自行处理焦点、方向键、Esc 和屏幕边缘避让。 -->
        <DropdownMenuContent class="annotation-group-menu" side="bottom" align="start" :side-offset="8" :collision-padding="8" :aria-label="t(name)" @pointerdown.stop @pointermove.stop @contextmenu.stop.prevent @keydown.stop>
          <DropdownMenuItem v-for="item in items" :key="item.tool" class="annotation-group-item" :disabled="disabled" @select="open = false; emit('select', item.tool)">
            <component :is="item.icon" :size="16" /><span>{{ t(item.name) }}</span><span v-if="active && selected === item.tool" class="chosen">✓</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenuPortal>
    </DropdownMenuRoot>
  </div>
</template>

<style scoped>
.tool-group { display: inline-flex; align-items: center; }
.tool-group .group-trigger { min-width: 24px; width: 24px; padding: 2px; }
</style>

<!-- 菜单被 Portal 移到 body，显式全局类不依赖组件根节点转发 scoped 标记。 -->
<style>
.annotation-group-menu { z-index:2000; isolation:isolate; min-width:156px; max-height:var(--reka-dropdown-menu-content-available-height); overflow-y:auto; padding:4px; border:1px solid #d8dee4; border-radius:6px; background-color:#fff; color:#24292f; opacity:1; box-shadow:0 4px 16px #0003; font:13px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
.annotation-group-menu .annotation-group-item { display:flex; align-items:center; gap:8px; min-height:32px; padding:4px 8px; border-radius:4px; cursor:pointer; outline:none; }
.annotation-group-menu .annotation-group-item[data-highlighted] { background:#eff6ff; color:#2563eb; }
.annotation-group-menu .annotation-group-item[data-disabled] { opacity:.45; pointer-events:none; }
.annotation-group-menu .chosen { margin-left:auto; color:#2563eb; }
[data-theme=dark] .annotation-group-menu { background-color:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .annotation-group-menu .annotation-group-item[data-highlighted] { background:#303f56; color:#93c5fd; }
</style>
