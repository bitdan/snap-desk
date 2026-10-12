<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, onScroll, type ScrollView } from '@/lib/api'

const controls = new URLSearchParams(location.search).get('window') === 'scroll-controls'
const view = ref<ScrollView | null>(null)
const busy = ref(false)
const error = ref('')
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
  busy.value = true
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
    <div v-if="!controls" class="canvas">
      <div v-if="view?.preview" class="image" :style="{ aspectRatio: `${view.width} / ${view.height}`, '--image-ratio': view.width / view.height }">
        <img :src="view.preview" alt="正在拼接的长截图" />
        <div class="current-frame" :style="marker" aria-label="当前采集画面的位置" />
      </div>
      <span v-else>准备采集…</span>
    </div>
    <footer v-if="controls">
      <div><button :disabled="busy || !view" @click="finish(false)">结束并保存</button><button :disabled="busy || !view" @click="finish(true)">取消</button></div>
      <p v-if="error" role="alert" :title="error">{{ error }}</p>
    </footer>
  </main>
</template>

<style scoped>
.live-preview { height: 100vh; display: flex; flex-direction: column; padding: 8px; gap: 8px; background: #fafafa; border: 2px solid #8b5cf6; color: #27272a; font-size: 12px; overflow: hidden; }
.canvas { flex: 1; min-height: 0; display: flex; justify-content: center; align-items: flex-end; overflow: hidden; background: #e4e4e7; container-type: size; }
.image { position: relative; width: min(100cqw, calc(100cqh * var(--image-ratio))); flex: none; }
img { display: block; width: 100%; height: 100%; }
.current-frame { position: absolute; left: 0; right: 0; border: 2px solid #84cc16; background: #84cc1610; pointer-events: none; }
footer { flex: none; display: grid; gap: 6px; }
footer div { display: flex; gap: 6px; }
button { flex: 1; border: 1px solid #d4d4d8; background: white; padding: 5px 2px; border-radius: 4px; }
button:first-child { background: #7c3aed; color: white; border-color: #7c3aed; }
button:disabled { opacity: .5; }
p { color: #b91c1c; overflow-wrap: anywhere; }
.controls { padding: 6px; border: 1px solid #d4d4d8; justify-content: center; }
.controls footer { display: flex; align-items: center; gap: 6px; }
.controls footer div { flex: 1; white-space: nowrap; }
.controls p { max-width: 110px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
