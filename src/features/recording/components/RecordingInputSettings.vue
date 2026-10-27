<script setup lang="ts">
import { t } from '@/shared/i18n/i18n'
import type { InputEffects } from '@/features/recording/recording'

const model = defineModel<InputEffects>({ required: true })
defineProps<{ disabled: boolean; inline?: boolean }>()
const toggles = [
  ['keyboard', '按键提示'], ['clicks', '点击光圈'],
  ['movement', '鼠标移动尾迹'], ['drag', '按住拖动轨迹'], ['wheel', '滚轮方向提示'],
] as const
const colors = [['leftColor', '左键 / 移动 / 滚轮'], ['rightColor', '右键 / 拖动'], ['doubleColor', '双击']] as const
function hex(color: [number, number, number]): string {
  return `#${color.map(value => value.toString(16).padStart(2, '0')).join('')}`
}
function changeColor(field: 'leftColor' | 'rightColor' | 'doubleColor', event: Event) {
  const value = (event.target as HTMLInputElement).value
  if (!/^#[0-9a-f]{6}$/i.test(value)) return
  model.value[field] = [parseInt(value.slice(1, 3), 16), parseInt(value.slice(3, 5), 16), parseInt(value.slice(5, 7), 16)]
}
</script>

<template>
  <section class="input-effects" :class="{ inline }">
    <label :title='t("仅显示快捷键、功能键与鼠标操作，不记录普通文字")' v-for="[field, label] in toggles" :key="field"><input v-model="model[field]" type="checkbox" :disabled="disabled" /> {{ t(label) }}</label>
    <template v-if="toggles.some(([field]) => model[field])">
      <label>{{ t("效果尺寸") }} <input v-model.number="model.size" type="number" min="12" max="64" :disabled="disabled" /> {{ t("像素") }}</label>
      <label>{{ t("持续时间") }} <input v-model.number="model.durationMs" type="number" min="200" max="2000" step="100" :disabled="disabled" /> {{ t("毫秒") }}</label>
      <label>{{ t("按键位置") }} <select v-model="model.position" :disabled="disabled"><option value="topLeft">{{ t("左上角") }}</option><option value="bottomLeft">{{ t("左下角") }}</option><option value="bottomRight">{{ t("右下角") }}</option></select></label>
      <label v-for="[field, label] in colors" :key="field">{{ t(label) }} <input :value="hex(model[field])" type="color" :disabled="disabled" @input="changeColor(field, $event)" /></label>
    </template>
  </section>
</template>

<style scoped>
.input-effects { font-size: 12px; }
.input-effects.inline { display: inline-flex; align-items: center; gap: 8px; }
.input-effects.inline label { margin-top: 0; white-space: nowrap; }
summary { cursor: pointer; font-weight: 600; }
label { display: flex; align-items: center; gap: 8px; margin-top: 6px; }
input[type="number"] { width: 76px; }
input[type="color"] { width: 36px; height: 24px; padding: 0; }
p { margin: 6px 0; font-size: 12px; color: #52525b; }
</style>
