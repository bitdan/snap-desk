<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Pin, PinOff, Lock, Unlock, Minus, Plus, Copy, Save, EyeOff, X, ChevronDown, ChevronUp } from 'lucide-vue-next'
import { api, type PinFrame, type PinView, type PinAction, type PinTransform } from '@/lib/api'
import { canEditPin, stepZoom } from '@/lib/pins'
import layout from '@/lib/pin-layout.json'

const frame = ref<PinFrame | null>(null)
const busy = ref(false)
const error = ref('')
const message = ref('')
const controls = computed(() => frame.value?.view.toolbarExpanded ?? true)
const focused = ref(document.hasFocus())
// 图片和底栏是两个网格区域；阴影占用外边距，底栏不再浮在图片上。
const shellStyle = computed(() => ({
  padding: `${layout.padding}px`, gap: `${layout.gap}px`,
  gridTemplateRows: `minmax(0, 1fr) ${controls.value ? layout.expandedHeight : layout.collapsedHeight}px`,
}))
const stageStyle = computed(() => ({
  cursor: editable.value ? 'move' : 'default', borderWidth: `${layout.border}px`,
  width: frame.value ? `min(100%, ${frame.value.width * frame.value.view.zoom + layout.border * 2}px)` : '100%',
  height: frame.value ? `min(100%, ${frame.value.height * frame.value.view.zoom + layout.border * 2}px)` : '100%',
}))
const editable = computed(() => canEditPin(frame.value?.view, busy.value))
const transforms: { action: PinTransform; label: string }[] = [
  { action: 'rotateLeft', label: '左转' }, { action: 'rotateRight', label: '右转' },
  { action: 'flipHorizontal', label: '水平翻转' }, { action: 'flipVertical', label: '垂直翻转' },
  { action: 'grayscale', label: '灰度' }, { action: 'invert', label: '反色' }, { action: 'reset', label: '恢复原图' },
]
/** 所有系统操作经过 API 层；失败保留当前图片并允许重试。 */
async function run(task: () => Promise<void>) {
  if (busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function load() { await run(async () => { frame.value = await api.currentPin() }) }
async function setView(patch: Partial<PinView>) {
  if (!frame.value) return
  await run(async () => { frame.value!.view = await api.setPinView({ ...frame.value!.view, ...patch }) })
}
function toggleControls() { void setView({ toolbarExpanded: !controls.value }) }
function updateFocus() { focused.value = document.hasFocus() }
async function action(value: PinAction) {
  await run(async () => {
    const result = await api.pinAction(value)
    message.value = value === 'copy' ? '已复制' : value === 'save' ? result ? '已保存 PNG' : '已取消保存' : ''
  })
}
async function transform(value: PinTransform) {
  if (!editable.value) return
  await run(async () => {
    frame.value = await api.transformPin(value)
    // 旋转可能交换宽高，按现有比例重新适配窗口；像素结果已由 Rust 合成。
    frame.value.view = await api.setPinView(frame.value.view)
  })
}
function zoom(direction: number) {
  if (editable.value) void setView({ zoom: stepZoom(frame.value!.view.zoom, direction) })
}
function wheel(event: WheelEvent) { if (event.deltaY !== 0) zoom(event.deltaY < 0 ? 1 : -1) }
function drag(event: PointerEvent) {
  if (event.button !== 0 || !editable.value) return
  void api.dragPin().catch(e => { error.value = String(e) })
}
async function chooseTransform(event: Event) {
  const select = event.target as HTMLSelectElement
  const item = transforms.find(item => item.action === select.value)
  if (item) await transform(item.action)
  select.value = ''
}
async function opacity(event: Event) {
  const input = event.target as HTMLInputElement
  await setView({ opacity: Number(input.value) / 100 })
  input.value = String(Math.round((frame.value?.view.opacity ?? 1) * 100))
}
function key(event: KeyboardEvent) {
  if (event.repeat) return
  const command = event.ctrlKey || event.metaKey
  const key = event.key.toLowerCase()
  if (command && ['c', 's', 'w'].includes(key)) {
    event.preventDefault(); void action(key === 'c' ? 'copy' : key === 's' ? 'save' : 'close'); return
  }
  if (key === 'escape') { event.preventDefault(); void action('hide'); return }
  if ((event.target as HTMLElement).closest('button, input, select')) return
  if (key === 'tab') { event.preventDefault(); toggleControls() }
  else if (key === '+' || key === '=') { event.preventDefault(); zoom(1) }
  else if (key === '-') { event.preventDefault(); zoom(-1) }
}
onMounted(() => {
  window.addEventListener('keydown', key)
  window.addEventListener('focus', updateFocus); window.addEventListener('blur', updateFocus)
  updateFocus(); void load()
})
onUnmounted(() => {
  window.removeEventListener('keydown', key)
  window.removeEventListener('focus', updateFocus); window.removeEventListener('blur', updateFocus)
})
</script>

<template>
  <div class="pin-shell" :class="{ 'is-focused': focused }" :style="shellStyle" @contextmenu.prevent="toggleControls">
    <div class="pin-stage" :style="stageStyle" @pointerdown="drag" @wheel.prevent="wheel">
      <div v-if="frame" class="pin-canvas">
        <img :src="frame.dataUrl" alt="桌面贴图" draggable="false" :style="{ width: `${frame.width * frame.view.zoom}px`, height: `${frame.height * frame.view.zoom}px`, opacity: frame.view.opacity }" @error="error = '贴图预览加载失败，可关闭后重新贴图。'" />
      </div>
    </div>
    <section class="pin-controls" :class="{ collapsed: !controls }" aria-label="贴图工具" @contextmenu.stop.prevent>
      <template v-if="frame && controls">
        <div class="pin-row" role="toolbar" aria-label="贴图操作">
          <button :disabled="busy" :aria-pressed="frame.view.topmost" title="置顶开关" aria-label="置顶开关" @click="setView({ topmost: !frame.view.topmost })"><component :is="frame.view.topmost ? Pin : PinOff" :size="16" /></button>
          <button :disabled="busy" :aria-pressed="frame.view.locked" :title="frame.view.locked ? '解锁' : '锁定'" :aria-label="frame.view.locked ? '解锁' : '锁定'" @click="setView({ locked: !frame.view.locked })"><component :is="frame.view.locked ? Lock : Unlock" :size="16" /></button>
          <span class="pin-divider" />
          <button :disabled="!editable" title="缩小" aria-label="缩小" @click="zoom(-1)"><Minus :size="16" /></button>
          <button :disabled="!editable" title="恢复 100% 查看比例" @click="setView({ zoom: 1 })">{{ Math.round(frame.view.zoom * 100) }}%</button>
          <button :disabled="!editable" title="放大" aria-label="放大" @click="zoom(1)"><Plus :size="16" /></button>
          <span class="pin-divider" />
          <button :disabled="busy" title="复制 Ctrl/⌘+C" aria-label="复制" @click="action('copy')"><Copy :size="16" /></button>
          <button :disabled="busy" title="保存 PNG Ctrl/⌘+S" aria-label="保存 PNG" @click="action('save')"><Save :size="16" /></button>
          <button :disabled="busy" title="隐藏 Esc" aria-label="隐藏" @click="action('hide')"><EyeOff :size="16" /></button>
          <button :disabled="busy" title="关闭 Ctrl/⌘+W" aria-label="关闭" @click="action('close')"><X :size="16" /></button>
          <button :disabled="busy" title="收起工具，右键重新展开" aria-label="收起工具" @click="toggleControls"><ChevronDown :size="16" /></button>
        </div>
        <div class="pin-row pin-options">
          <select aria-label="图片处理" :disabled="!editable" @change="chooseTransform"><option value="">图片处理</option><option v-for="item in transforms" :key="item.action" :value="item.action">{{ item.label }}</option></select>
          <label v-if="frame.transparencySupported" class="pin-opacity">不透明度 <input type="range" min="20" max="100" step="5" :value="Math.round(frame.view.opacity * 100)" :disabled="busy" @change="opacity" /><span>{{ Math.round(frame.view.opacity * 100) }}%</span></label>
          <span class="pin-dimensions">{{ frame.width }} × {{ frame.height }} px</span>
        </div>
      </template>
      <div v-else-if="!frame" class="pin-row"><span>{{ busy ? '正在加载…' : '图片未加载' }}</span><button :disabled="busy" @click="load">重试</button><button :disabled="busy" @click="action('close')">关闭</button></div>
      <button v-else class="pin-toggle" :disabled="busy" title="展开贴图工具（右键）" @click="toggleControls"><ChevronUp :size="16" />工具</button>
      <p v-if="error" role="alert" class="pin-status pin-error" :title="error">{{ error }}</p>
      <p v-else-if="message" role="status" class="pin-status" :title="message">{{ message }}</p>
      <p v-else-if="controls" class="pin-status">拖动移动 · 滚轮缩放 · 右键收起 · Esc 隐藏</p>
    </section>
  </div>
</template>

<style scoped>
.pin-shell { position: fixed; inset: 0; display: grid; justify-items: center; color: #374151; user-select: none; }
.pin-stage { min-width: 0; min-height: 0; align-self: center; overflow: auto; border-style: solid; border-color: #a1a1aa; box-shadow: 0 2px 8px #0005; transition: border-color .12s, box-shadow .12s; }
.is-focused .pin-stage { border-color: #a78bfa; box-shadow: 0 0 0 1px #a78bfa66, 0 2px 8px #0005; }
.pin-canvas { display: flex; align-items: center; justify-content: center; min-width: 100%; min-height: 100%; width: max-content; height: max-content; }
.pin-canvas img { flex: none; max-width: none; }
.pin-controls { min-width: 0; width: max-content; max-width: 100%; height: 100%; justify-self: end; overflow: hidden; padding: 4px 6px; border: 1px solid #d4d4d8; border-radius: 7px; background: #fffffff5; box-shadow: 0 2px 6px #0002; font-size: 11px; }
.pin-row { display: flex; align-items: center; gap: 3px; height: 34px; overflow-x: auto; overflow-y: hidden; scrollbar-width: thin; white-space: nowrap; }
.pin-row > * { flex-shrink: 0; }
.pin-controls button { display: inline-flex; align-items: center; justify-content: center; gap: 4px; height: 26px; min-width: 26px; padding: 3px 5px; border-radius: 4px; }
.pin-controls button:hover { background: #f3f0ff; }
.pin-controls button:disabled { opacity: .45; }
.pin-controls button[aria-pressed=true] { color: #7c3aed; background: #ede9fe; }
.pin-controls button:focus-visible, .pin-controls select:focus-visible { outline: 2px solid #a78bfa; outline-offset: -2px; }
.pin-divider { height: 14px; width: 1px; background: #e4e4e7; margin: 0 3px; }
.pin-options { gap: 10px; }
.pin-options select { background: transparent; max-width: 100px; height: 26px; }
.pin-opacity { display: flex; align-items: center; gap: 4px; }
.pin-opacity input { width: 64px; accent-color: #8b5cf6; }
.pin-dimensions { color: #71717a; }
.pin-status { line-height: 16px; margin: 0; font-size: 10px; color: #71717a; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pin-error { color: #b91c1c; }
.collapsed { display: flex; align-items: center; gap: 8px; }
.collapsed .pin-status { min-width: 0; }
.pin-toggle { flex-shrink: 0; }
</style>
