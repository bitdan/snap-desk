<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Pin, PinOff, PanelsTopLeft, X, Copy, File, Type } from 'lucide-vue-next'
import { api } from '@/services/api'
import { t } from '@/shared/i18n/i18n'
import type { ClipboardItem } from '@/features/clipboard/clipboard'
import { createShelfGesture } from '../shelf-gesture'
import { shelfPhotoSize } from '../shelf-layout'

const props = defineProps<{ item: ClipboardItem; kept: boolean; busy: boolean }>()
const emit = defineEmits<{ copy: []; drag: []; keep: []; remove: []; pin: []; error: [message: string] }>()
const thumbnail = ref('')
const photo = computed(() => shelfPhotoSize(props.item.width, props.item.height))
const frameStyle = computed(() => props.item.kind === 'image'
  ? { width: `${photo.value.width + 10}px`, height: `${photo.value.height + 10}px` }
  : { width: '154px', height: '110px' })
const tilt = computed(() => `${[-1.2, 0.8, -0.6, 1.4, -0.9][props.item.id % 5]}deg`)
let disposed = false
async function load() {
  if (props.item.kind !== 'image') return
  try { const value = await api.clipboardThumbnail(props.item.id); if (!disposed) thumbnail.value = value }
  catch (e) { if (!disposed) emit('error', String(e)) }
}
const gesture = createShelfGesture(() => { if (!props.busy) emit('copy') }, () => { if (!props.busy) emit('drag') })
watch(() => props.busy, value => { if (value) gesture.cancel() })
onMounted(load)
onUnmounted(() => { disposed = true; gesture.cancel() })
</script>
<template>
  <article class="shelf-card" :class="{ kept }" :style="{ '--rest-tilt': tilt }">
    <span class="shelf-clip" aria-hidden="true"><span /></span>
    <div class="shelf-frame" :class="{ 'shelf-note-frame': item.kind !== 'image', 'shelf-file-frame': item.kind === 'file' }" :style="frameStyle">
      <button class="shelf-content" :disabled="busy" :title="t('单击复制，拖动分享')" :aria-label="t('复制：{0}', { 0: item.preview })"
        @pointerdown="gesture.down" @pointermove="gesture.move" @pointerup="gesture.cancel" @pointercancel="gesture.cancel" @lostpointercapture="gesture.cancel" @click="gesture.click" @dragstart.prevent>
        <img v-if="thumbnail" :src="thumbnail" :alt="t('图片')" draggable="false" />
        <span v-else-if="item.kind === 'image'">{{ t('正在加载…') }}</span>
        <span v-else class="shelf-text"><File v-if="item.kind === 'file'" :size="16" /><Type v-else :size="16" /><span>{{ item.preview }}</span></span>
      </button>
      <button class="shelf-remove" :disabled="busy" :title="t('从架子移除，保留历史')" :aria-label="t('从架子移除，保留历史')" @click="emit('remove')"><X :size="12" /></button>
      <div class="shelf-card-actions">
        <button :disabled="busy" :title="t('复制')" :aria-label="t('复制')" @click="emit('copy')"><Copy :size="14" /></button>
        <button :disabled="busy" :title="t(kept ? '取消固定' : '固定在架上')" :aria-label="t(kept ? '取消固定' : '固定在架上')" :aria-pressed="kept" @click="emit('keep')"><PinOff v-if="kept" :size="14" /><Pin v-else :size="14" /></button>
        <button :disabled="busy" :title="t('取下为贴图')" :aria-label="t('取下为贴图')" @click="emit('pin')"><PanelsTopLeft :size="14" /></button>
      </div>
    </div>
  </article>
</template>
<style scoped>
.shelf-card { position: absolute; width: 156px; display: flex; justify-content: center; padding-top: 18px; transform-origin: 50% 9px; transform: rotate(var(--rest-tilt)); animation: hang .6s ease-out; transition: transform .22s ease; }
.shelf-card:hover, .shelf-card:focus-within { z-index: 2; transform: rotate(0deg) translateY(2px); }
.shelf-frame { position: relative; padding: 4px; border: 1px solid #ffffffce; border-radius: 13px; background: linear-gradient(145deg, #ffffffe0, #f4f4f580); box-shadow: inset 0 0 0 1px #ffffff70, 0 4px 9px #0f172a26, 0 12px 22px -12px #0f172a38; }
.shelf-clip { position: absolute; top: 0; left: calc(50% - 4px); width: 8px; height: 25px; z-index: 3; border-radius: 3px; border: 1px solid #ffffffa6; background: linear-gradient(90deg, #9d9fa2, #f3f4f5 35%, #d4d6d8 65%, #96999c); box-shadow: 1px 2px 3px #0003, inset 0 0 0 .5px #0002; }
.shelf-clip > span { position: absolute; left: 1px; right: 1px; top: 8px; height: 1px; background: #5c606675; box-shadow: 0 1px #fff9; }
.kept .shelf-clip { background: linear-gradient(90deg, #414a55, #89939f 40%, #5d6875 70%, #35404d); border-color: #afb8c1; }
.shelf-content { width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; overflow: hidden; border-radius: 9px; background: transparent; color: #475569; text-align: left; cursor: grab; touch-action: none; font-size: 12px; }
.shelf-content:active { cursor: grabbing; }
.shelf-content img { width: 100%; height: 100%; object-fit: contain; pointer-events: none; }
.shelf-note-frame .shelf-content { background: #fff8da; }
.shelf-file-frame .shelf-content { background: #edf2fa; }
.shelf-text { padding: 12px 10px 8px; width: 100%; font-size: 12px; line-height: 1.5; white-space: pre-wrap; overflow-wrap: anywhere; height: 100%; overflow: hidden; }
.shelf-text > svg { color: #8b7e58; }
.shelf-text > span { display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; margin-top: 5px; }
.shelf-card-actions { position: absolute; left: 50%; top: calc(100% + 7px); transform: translate(-50%, -3px); display: flex; padding: 3px; gap: 2px; border: 1px solid #ffffffcf; border-radius: 18px; background: #fafafaf2; box-shadow: 0 3px 12px #0002; opacity: 0; pointer-events: none; transition: opacity .15s, transform .15s; }
.shelf-card-actions::before { content: ''; position: absolute; bottom: 100%; left: 0; width: 100%; height: 9px; }
.shelf-remove { position: absolute; top: -7px; left: -7px; width: 24px; height: 24px; display: grid; place-items: center; border-radius: 50%; border: 1px solid #fff; background: #f8fafcf2; color: #475569; box-shadow: 0 2px 5px #0002; opacity: 0; pointer-events: none; }
.shelf-card:hover .shelf-card-actions, .shelf-card:focus-within .shelf-card-actions { opacity: 1; pointer-events: auto; transform: translate(-50%, 0); }
.shelf-card:hover .shelf-remove, .shelf-card:focus-within .shelf-remove { opacity: 1; pointer-events: auto; }
.shelf-card-actions button { display: grid; place-items: center; width: 27px; height: 25px; border-radius: 50%; color: #475569; }
.shelf-card-actions button:hover, .shelf-remove:hover { background: #e2e8f0; }
.shelf-card-actions button[aria-pressed=true] { color: #2563eb; background: #e8efff; }
button:focus-visible { outline: 2px solid #2563eb; outline-offset: 2px; }
@keyframes hang { 0% { opacity: 0; transform: translateY(-24px) rotate(-5deg); } 55% { opacity: 1; transform: translateY(3px) rotate(2deg); } 80% { transform: translateY(-1px) rotate(-1deg); } 100% { transform: rotate(var(--rest-tilt)); } }
@media (prefers-reduced-motion: reduce) { .shelf-card { animation: none; transition: none; } .shelf-card-actions { transition: none; } }
</style>
