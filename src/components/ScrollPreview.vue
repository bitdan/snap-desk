<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Save, X, AlertCircle } from 'lucide-vue-next'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { resolveWindowKind } from '@/lib/window-kind'
import { api, onScroll, type ScrollView } from '@/lib/api'

const controls = resolveWindowKind(isTauri() ? getCurrentWindow().label : null, location.search) === 'scroll-controls'
const view = ref<ScrollView | null>(null)
const busy = ref(false)
const error = ref('')
const cancelling = ref(false)
const status = computed(() => busy.value ? (cancelling.value ? '正在取消…' : '正在保存…')
  : !view.value ? '准备中…' : !view.value.collecting ? '采集结束' : '手动采集')
const hint = computed(() => error.value || view.value?.message || '滚动后稍作停顿，等待拼接')
let off: (() => void) | undefined
let disposed = false
let updates = 0
function receive(next: ScrollView) {
  if (view.value && (next.id < view.value.id || (next.id === view.value.id && next.revision < view.value.revision))) return
  updates++
  if (next.id !== view.value?.id) { busy.value = false; error.value = '' }
  view.value = next
}
// 缩略图与当前帧都按整张长图的高度计算，绿色框随追加向下移动。
const marker = computed(() => {
  const frame = view.value
  if (!frame) return {}
  const fraction = Math.min(1, frame.frameHeight / frame.height)
  return { top: `${(1 - fraction) * 100}%`, height: `${fraction * 100}%` }
})
async function finish(cancel: boolean) {
  if (!view.value || busy.value) return
  busy.value = true; cancelling.value = cancel; error.value = ''
  try { await api.controlScroll(view.value.id, cancel) }
  catch (e) { error.value = String(e); busy.value = false }
}
onMounted(async () => {
  try {
    off = await onScroll(receive)
    if (disposed) { off(); return }
    const revision = updates
    const current = await api.currentScroll()
    if (!disposed && updates === revision && current) receive(current)
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.() })
</script>

<template>
  <main class="live-preview" :class="{ controls }">
    <template v-if="!controls">
      <header><strong>长截图</strong><span v-if="view" :title="`已拼接 ${view.frames} 帧`">{{ view.width }} × {{ view.height }}</span></header>
      <div class="canvas">
        <div v-if="view?.preview" class="image" :style="{ aspectRatio: `${view.width} / ${view.height}`, '--image-ratio': view.width / view.height }">
          <img :src="view.preview" alt="正在拼接的长截图" />
          <div class="current-frame" :style="marker" title="当前采集画面" aria-label="当前采集画面的位置" />
        </div>
        <span v-else>准备采集…</span>
      </div>
      <p class="preview-hint" :class="{ error }" :role="error ? 'alert' : undefined" :title="hint">{{ hint }}</p>
    </template>
    <footer v-else>
      <div class="progress" :title="hint">
        <span class="status"><i :class="{ stopped: busy || !view?.collecting }" />{{ status }}</span>
        <small v-if="view">{{ view.frames }} 帧 · {{ view.height }} px</small>
      </div>
      <span v-if="error" class="error-indicator" role="alert" :title="error" :aria-label="error"><AlertCircle :size="16" /></span>
      <button class="save" :disabled="busy || !view" title="结束采集，保存并复制长截图" @click="finish(false)"><Save :size="15" />保存</button>
      <button class="cancel" :disabled="busy || !view" title="取消长截图（已保留的原始帧不会删除）" aria-label="取消长截图" @click="finish(true)"><X :size="17" /></button>
    </footer>
  </main>
</template>

<style scoped>
.live-preview { height: 100vh; display: flex; flex-direction: column; padding: 6px; gap: 6px; background: #fafafa; border: 1px solid #d4d4d8; color: #27272a; font-size: 12px; overflow: hidden; }
header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 4px; flex: none; padding: 2px; }
header span { color: #71717a; font-size: 10px; }
.canvas { flex: 1; min-height: 0; display: flex; justify-content: center; align-items: flex-end; overflow: hidden; background: #e4e4e7; border-radius: 3px; container-type: size; }
.image { position: relative; width: min(100cqw, calc(100cqh * var(--image-ratio))); flex: none; }
img { display: block; width: 100%; height: 100%; }
.current-frame { position: absolute; left: 0; right: 0; border: 2px solid #84cc16; background: #84cc1610; pointer-events: none; }
.preview-hint { flex: none; margin: 0; color: #71717a; font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.controls { justify-content: center; padding: 5px 8px; }
footer { display: flex; align-items: center; gap: 7px; min-width: 0; }
.progress { flex: 1; min-width: 0; display: grid; gap: 1px; }
.status { display: flex; align-items: center; gap: 5px; white-space: nowrap; }
.status i { width: 6px; height: 6px; border-radius: 50%; background: #7c3aed; flex: none; }.status i.stopped { background: #a1a1aa; }
small { color: #71717a; font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
button { flex: none; display: flex; align-items: center; justify-content: center; gap: 5px; height: 30px; border-radius: 5px; padding: 0 9px; }
.save { background: #7c3aed; color: white; }.cancel { padding: 0 5px; }.cancel:hover:not(:disabled) { background: #fee2e2; color: #b91c1c; }
button:disabled { opacity: .5; }.error, .error-indicator { color: #b91c1c; }.error-indicator { display: flex; }
button:focus-visible { outline: 2px solid #a78bfa; outline-offset: 1px; }
</style>
