<script setup lang="ts">
import { t } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
import { ref, watch, type Component } from 'vue'
import { Check } from 'lucide-vue-next'
import { DropdownMenuRoot, DropdownMenuTrigger, DropdownMenuPortal, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator } from 'reka-ui'

/** 贴图工具栏的图标下拉按钮；菜单经 Portal 绘制，自行处理焦点、方向键、Esc 和边缘避让。 */
export interface PinMenuItem { value: string; label: string; checked?: boolean; disabled?: boolean; separator?: boolean }
const props = defineProps<{ icon: Component; label: string; items: PinMenuItem[]; disabled?: boolean; side?: 'top' | 'bottom' }>()
const emit = defineEmits<{ select: [value: string]; openChange: [open: boolean] }>()
const open = ref(false)
useEscapeLayer(() => open.value, () => { open.value = false })
watch(open, value => emit('openChange', value))
watch(() => props.disabled, disabled => { if (disabled) open.value = false })
function dismissKey(event: KeyboardEvent) { handleEscape(event, () => { open.value = false }) }
</script>

<template>
  <DropdownMenuRoot v-model:open="open">
    <DropdownMenuTrigger :disabled="disabled" :title="t(label)" :aria-label="t(label)"><component :is="icon" :size="17" /></DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent class="annotation-group-menu pin-menu" :side="side ?? 'bottom'" align="end" :side-offset="8" :collision-padding="8" :aria-label="t(label)" @pointerdown.stop @contextmenu.stop.prevent @keydown.stop="dismissKey">
        <template v-for="item in items" :key="item.value">
          <DropdownMenuSeparator v-if="item.separator" class="pin-menu-separator" />
          <DropdownMenuItem class="annotation-group-item" :disabled="disabled || item.disabled" @select="open = false; emit('select', item.value)">
            <span>{{ t(item.label) }}</span><Check v-if="item.checked" :size="14" class="chosen" aria-hidden="true" />
          </DropdownMenuItem>
        </template>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>

<style>
.pin-menu .pin-menu-separator { height: 1px; margin: 4px; background: #e4e4e7; }
[data-theme=dark] .pin-menu .pin-menu-separator { background: #39424e; }
</style>
