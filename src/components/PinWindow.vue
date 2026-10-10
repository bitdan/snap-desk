<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, type PinFrame, type PinView, type PinAction, type PinTransform } from '@/lib/api'
import { canEditPin, stepZoom } from '@/lib/pins'

const frame = ref<PinFrame | null>(null)
const busy = ref(false)
const error = ref('')
const message = ref('')
const controls = ref(true)
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
  try { await task() } catch (e) { error.value = String(e); controls.value = true }
  finally { busy.value = false }
}
async function load() { await run(async () => { frame.value = await api.currentPin() }) }
async function setView(patch: Partial<PinView>) {
  if (!frame.value) return
  await run(async () => { frame.value!.view = await api.setPinView({ ...frame.value!.view, ...patch }) })
}
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
  void api.dragPin().catch(e => { error.value = String(e); controls.value = true })
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
  if (key === 'tab') { event.preventDefault(); controls.value = !controls.value }
  else if (key === '+' || key === '=') { event.preventDefault(); zoom(1) }
  else if (key === '-') { event.preventDefault(); zoom(-1) }
}
onMounted(() => { window.addEventListener('keydown', key); void load() })
onUnmounted(() => window.removeEventListener('keydown', key))
</script>

<template>
  <div class="pin-shell" @contextmenu.prevent="controls = !controls">
    <div class="pin-stage" :style="{ cursor: editable ? 'move' : 'default' }" @pointerdown="drag" @wheel.prevent="wheel">
      <div v-if="frame" class="pin-canvas">
        <img :src="frame.dataUrl" alt="桌面贴图" draggable="false" :style="{ width: `${frame.width * frame.view.zoom}px`, height: `${frame.height * frame.view.zoom}px`, opacity: frame.view.opacity }" @error="error = '贴图预览加载失败，可关闭后重新贴图。'; controls = true" />
      </div>
    </div>
    <button v-if="!controls" class="pin-toggle" title="展开贴图工具（右键）" @click="controls = true">工具</button>
    <section v-if="controls" class="pin-controls" aria-label="贴图工具" @contextmenu.stop.prevent>
      <div class="pin-row"><strong>{{ frame?.label ?? '贴图' }}</strong><span v-if="frame">{{ frame.width }} × {{ frame.height }}</span><button title="收起工具，右键重新展开" @click="controls = false">收起</button></div>
      <template v-if="frame">
        <div class="pin-row">
          <button :disabled="busy" :aria-pressed="frame.view.topmost" @click="setView({ topmost: !frame.view.topmost })">{{ frame.view.topmost ? '已置顶' : '置顶' }}</button>
          <button :disabled="busy" :aria-pressed="frame.view.locked" @click="setView({ locked: !frame.view.locked })">{{ frame.view.locked ? '解锁' : '锁定' }}</button>
          <button :disabled="!editable" aria-label="缩小" @click="zoom(-1)">−</button>
          <button :disabled="!editable" title="恢复 100% 查看比例" @click="setView({ zoom: 1 })">{{ Math.round(frame.view.zoom * 100) }}%</button>
          <button :disabled="!editable" aria-label="放大" @click="zoom(1)">＋</button>
        </div>
        <label v-if="frame.transparencySupported" class="pin-row">不透明度 <input type="range" min="20" max="100" step="5" :value="Math.round(frame.view.opacity * 100)" :disabled="busy" @change="opacity" /><span>{{ Math.round(frame.view.opacity * 100) }}%</span></label>
        <div class="pin-row"><button v-for="item in transforms" :key="item.action" :disabled="!editable" @click="transform(item.action)">{{ item.label }}</button></div>
        <div class="pin-row"><button :disabled="busy" @click="action('copy')">复制</button><button :disabled="busy" @click="action('save')">保存 PNG</button><button :disabled="busy" @click="action('hide')">隐藏</button><button :disabled="busy" @click="action('close')">关闭</button></div>
        <small>拖动图片移动 · 滚轮缩放 · 右键收起工具<br />Esc 隐藏，可在主窗口“贴图”页找回</small>
      </template>
      <div v-else class="pin-row"><span>{{ busy ? '正在加载…' : '图片未加载' }}</span><button :disabled="busy" @click="load">重试</button><button :disabled="busy" @click="action('close')">关闭</button></div>
      <p v-if="error" role="alert" class="pin-error">{{ error }}</p><p v-else-if="message" role="status">{{ message }}</p>
    </section>
  </div>
</template>

<style scoped>
.pin-shell { position: fixed; inset: 0; color: #f4f4f5; user-select: none; }
.pin-stage { position: absolute; inset: 0; overflow: auto; cursor: move; }
.pin-canvas { display: flex; align-items: center; justify-content: center; min-width: 100%; min-height: 100%; width: max-content; height: max-content; }
.pin-canvas img { flex: none; max-width: none; }
.pin-controls { position: absolute; inset: 6px 6px auto; max-height: calc(100% - 12px); overflow: auto; padding: 8px; border: 1px solid #64748b; border-radius: 8px; background: #24292ff5; font-size: 11px; }
.pin-row { display: flex; align-items: center; flex-wrap: wrap; gap: 5px; margin-bottom: 6px; }
.pin-row strong { margin-right: auto; }
.pin-controls button, .pin-toggle { padding: 4px 7px; border-radius: 4px; background: #3e4650; }
.pin-controls button:hover, .pin-toggle:hover { background: #526071; }
.pin-controls button:disabled { opacity: .45; }
.pin-controls button[aria-pressed=true] { background: #2563eb; }
.pin-row input { width: 90px; }
.pin-controls small { color: #cbd5e1; font-size: 10px; }
.pin-controls p { overflow-wrap: anywhere; margin-top: 6px; }
.pin-error { color: #fca5a5; }
.pin-toggle { position: absolute; top: 6px; right: 6px; font-size: 11px; }
</style>
