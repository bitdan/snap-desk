<script setup lang="ts">
import type { InputEffects } from '@/lib/recording'

const model = defineModel<InputEffects>({ required: true })
defineProps<{ disabled: boolean }>()
const toggles = [
  ['keyboard', '快捷键 / 功能键提示'], ['clicks', '左键 / 右键 / 双击光圈'],
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
  <details class="input-effects">
    <summary>键鼠操作可视化</summary>
    <p>特效会写入视频。默认关闭；仅显示常用快捷键和功能键，不显示普通文字输入。暂停或结束时停止监听。</p>
    <label v-for="[field, label] in toggles" :key="field"><input v-model="model[field]" type="checkbox" :disabled="disabled" /> {{ label }}</label>
    <template v-if="toggles.some(([field]) => model[field])">
      <label>效果尺寸 <input v-model.number="model.size" type="number" min="12" max="64" :disabled="disabled" /> 像素</label>
      <label>持续时间 <input v-model.number="model.durationMs" type="number" min="200" max="2000" step="100" :disabled="disabled" /> 毫秒</label>
      <label>按键位置 <select v-model="model.position" :disabled="disabled"><option value="bottomLeft">左下角</option><option value="bottomRight">右下角</option></select></label>
      <label v-for="[field, label] in colors" :key="field">{{ label }} <input :value="hex(model[field])" type="color" :disabled="disabled" @input="changeColor(field, $event)" /></label>
    </template>
  </details>
</template>

<style scoped>
.input-effects { border-top: 1px solid #d4d4d8; padding-top: 8px; }
summary { cursor: pointer; font-weight: 600; }
label { display: flex; align-items: center; gap: 8px; margin-top: 6px; }
input[type="number"] { width: 76px; }
input[type="color"] { width: 36px; height: 24px; padding: 0; }
p { margin: 6px 0; font-size: 12px; color: #52525b; }
</style>
