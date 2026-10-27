<script setup lang="ts">
import { computed } from 'vue'
import { Keyboard, MousePointer2, Mouse, Route, MoveVertical, RotateCcw } from 'lucide-vue-next'
import { t } from '@/shared/i18n/i18n'
import { defaultInputEffects, type InputEffects } from '@/features/recording/recording'

const model = defineModel<InputEffects>({ required: true })
const props = defineProps<{ disabled: boolean }>()
const mouseToggles = [
  { field: 'clicks', label: '点击光圈', icon: Mouse },
  { field: 'movement', label: '鼠标移动尾迹', icon: MousePointer2 },
  { field: 'drag', label: '按住拖动轨迹', icon: Route },
  { field: 'wheel', label: '滚轮方向提示', icon: MoveVertical },
] as const
const colors = [
  { field: 'leftColor', label: '左键 / 移动 / 滚轮', enabled: () => model.value.clicks || model.value.movement || model.value.wheel },
  { field: 'rightColor', label: '右键 / 拖动', enabled: () => model.value.clicks || model.value.drag },
  { field: 'doubleColor', label: '双击', enabled: () => model.value.clicks },
] as const
const positions = [['topLeft', '左上角'], ['bottomLeft', '左下角'], ['bottomRight', '右下角']] as const
const anyEnabled = computed(() => model.value.keyboard || mouseToggles.some(({ field }) => model.value[field]))
function hex(color: [number, number, number]): string {
  return `#${color.map(value => value.toString(16).padStart(2, '0')).join('')}`
}
function changeColor(field: 'leftColor' | 'rightColor' | 'doubleColor', event: Event) {
  if (props.disabled) return
  const value = (event.target as HTMLInputElement).value
  if (!/^#[0-9a-f]{6}$/i.test(value)) return
  model.value[field] = [parseInt(value.slice(1, 3), 16), parseInt(value.slice(3, 5), 16), parseInt(value.slice(5, 7), 16)]
}
/** 用完整的新对象恢复默认，避免旧会话或颜色数组与当前草稿共享引用。 */
function reset() {
  if (!props.disabled) model.value = defaultInputEffects()
}
</script>

<template>
  <section class="input-effects" :aria-label="t('键鼠特效')">
    <header class="effects-header">
      <strong>{{ t('键鼠特效') }}</strong>
      <button type="button" class="effects-reset" :disabled="disabled" @click="reset"><RotateCcw :size="13" />{{ t('恢复默认') }}</button>
    </header>
    <div class="effects-group mouse-effects" role="group" :aria-label="t('鼠标效果')">
      <label v-for="{ field, label, icon } in mouseToggles" :key="field" class="effect-toggle">
        <component :is="icon" :size="15" aria-hidden="true" /><span>{{ t(label) }}</span>
        <input v-model="model[field]" type="checkbox" role="switch" :disabled="disabled" />
      </label>
    </div>
    <div class="effects-group">
      <label class="effect-toggle keyboard-toggle"><Keyboard :size="15" aria-hidden="true" /><span>{{ t('按键提示') }}</span><input v-model="model.keyboard" type="checkbox" role="switch" :disabled="disabled" /></label>
      <p class="effects-note">{{ t('仅显示快捷键、功能键与鼠标操作，不记录普通文字') }}</p>
      <div class="effect-setting position-setting">
        <span>{{ t('按键位置') }}</span>
        <div class="effect-positions" role="group" :aria-label="t('按键位置')">
          <button v-for="[value, label] in positions" :key="value" type="button" :aria-pressed="model.position === value" :disabled="disabled || !model.keyboard" @click="model.position = value">{{ t(label) }}</button>
        </div>
      </div>
    </div>
    <!-- 参数始终占据固定位置，开关只改变可用状态，避免菜单跳动和焦点丢失。 -->
    <div class="effects-group effect-parameters">
      <label class="effect-setting"><span>{{ t('效果尺寸') }}</span><input v-model.number="model.size" type="range" min="12" max="64" :disabled="disabled || !anyEnabled" /><output>{{ model.size }} px</output></label>
      <label class="effect-setting"><span>{{ t('持续时间') }}</span><input v-model.number="model.durationMs" type="range" min="200" max="2000" step="100" :disabled="disabled || !anyEnabled" /><output>{{ model.durationMs }} ms</output></label>
    </div>
    <div class="effects-group effect-colors" role="group" :aria-label="t('颜色')">
      <label v-for="{ field, label, enabled } in colors" :key="field" class="effect-color" :class="{ inactive: !enabled() }">
        <span>{{ t(label) }}</span><input :value="hex(model[field])" type="color" :disabled="disabled || !enabled()" @input="changeColor(field, $event)" />
      </label>
    </div>
  </section>
</template>

<style scoped>
.input-effects { --effect-border:#d8dee4; --effect-muted:#57606a; --effect-hover:#f0f3f6; --effect-track:#c2c9d1; font-size:13px; }
.effects-header { display:flex; align-items:center; justify-content:space-between; gap:12px; padding:0 2px 8px; }
.effects-header strong { font-size:13px; font-weight:600; }
.effects-reset { display:inline-flex; align-items:center; gap:4px; min-height:28px; padding:2px 5px; border:0; border-radius:4px; background:transparent; color:var(--effect-muted); font:inherit; font-size:12px; cursor:pointer; }
.effects-reset:hover:enabled { background:var(--effect-hover); color:inherit; }
.effects-group { padding:9px 0; border-top:1px solid var(--effect-border); }
.effects-group:last-child { padding-bottom:0; }
.mouse-effects { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:4px 8px; }
.input-effects .effect-toggle { display:flex; align-items:center; gap:7px; min-height:36px; margin:0; padding:4px; border-radius:4px; cursor:pointer; }
.effect-toggle:hover { background:var(--effect-hover); }
.effect-toggle > svg { flex:none; color:var(--effect-muted); }
.effect-toggle > span { flex:1; min-width:0; font-size:12px; line-height:1.4; }
/* 保留原生复选框的键盘与标签行为，只将外观绘制成开关。 */
.effect-toggle input[type=checkbox] { appearance:none; position:relative; flex:none; width:28px; height:16px; margin:0; border:1px solid transparent; border-radius:10px; background:var(--effect-track); cursor:pointer; transition:background .12s; }
.effect-toggle input[type=checkbox]::after { content:''; position:absolute; top:2px; left:2px; width:10px; height:10px; border-radius:50%; background:white; transition:transform .12s; }
.effect-toggle input[type=checkbox]:checked { background:#2563eb; }
.effect-toggle input[type=checkbox]:checked::after { transform:translateX(12px); }
.effects-note { margin:0 4px 10px; color:var(--effect-muted); font-size:12px; line-height:1.5; }
.input-effects .effect-setting { display:grid; grid-template-columns:80px minmax(0,1fr) 58px; align-items:center; gap:8px; min-height:34px; margin:0; padding:2px 4px; }
.effect-setting > span { font-size:12px; }
.effect-setting input[type=range] { width:100%; min-width:0; height:24px; margin:0; padding:0; border:0; accent-color:#2563eb; cursor:pointer; }
.effect-setting output { text-align:right; font-size:12px; font-variant-numeric:tabular-nums; }
.input-effects .position-setting { grid-template-columns:80px minmax(0,1fr); }
.effect-positions { display:flex; padding:2px; border:1px solid var(--effect-border); border-radius:5px; }
.effect-positions button { flex:1; min-width:0; min-height:26px; padding:3px; border:0; border-radius:3px; background:transparent; color:inherit; font:inherit; font-size:12px; cursor:pointer; }
.effect-positions button:hover:enabled { background:var(--effect-hover); }
.effect-positions button[aria-pressed=true]:enabled { background:#dbeafe; color:#1d4ed8; }
.effect-colors { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px 10px; }
.input-effects .effect-color { display:flex; align-items:center; gap:6px; min-height:28px; margin:0; font-size:12px; }
.effect-color input[type=color] { width:26px; height:26px; padding:2px; border:1px solid var(--effect-border); border-radius:5px; background:transparent; cursor:pointer; }
.effect-color input[type=color]::-webkit-color-swatch-wrapper { padding:0; }
.effect-color input[type=color]::-webkit-color-swatch { border:0; border-radius:2px; }
.effect-color.inactive { color:var(--effect-muted); }
.input-effects :disabled { opacity:.45; cursor:default; }
.input-effects :focus-visible { outline:2px solid #2563eb; outline-offset:2px; }
:global([data-theme=dark]) .input-effects { --effect-border:#39424e; --effect-muted:#a8b3c2; --effect-hover:#303944; --effect-track:#596574; }
:global([data-theme=dark]) .effect-positions button[aria-pressed=true]:enabled { background:#253e60; color:#93c5fd; }
@media (max-width:380px) { .mouse-effects { grid-template-columns:1fr; } .input-effects .effect-setting { grid-template-columns:72px minmax(0,1fr) 58px; } .input-effects .position-setting { grid-template-columns:72px minmax(0,1fr); } }
@media (prefers-reduced-motion:reduce) { .effect-toggle input[type=checkbox], .effect-toggle input[type=checkbox]::after { transition:none; } }
</style>
