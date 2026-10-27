<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, nextTick, onMounted, ref } from 'vue'
import ImageTranslation from './ImageTranslation.vue'
import { api, type PinFrame, type QrResult } from '@/lib/api'
import type { OcrResult } from '@/lib/ai'
import { ocrPolygon, selectedOcrText } from '@/lib/pin-recognition'

const props = defineProps<{ frame: PinFrame; mode: 'ocr' | 'qr' | 'translate' }>()
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)
const textArea = ref<HTMLTextAreaElement | null>(null)
const busy = ref(false), error = ref(''), message = ref('')
const ocr = ref<OcrResult | null>(null), qr = ref<QrResult | null>(null)
const selected = ref<string[]>([])
const translating = ref(props.mode === 'translate')
function backToRecognition() { translating.value = false; void nextTick(() => panel.value?.focus()) }
const pendingLink = ref<number | null>(null)
const text = computed(() => ocr.value ? selectedOcrText(ocr.value, selected.value) : '')
const lines = computed(() => [...(ocr.value?.lines ?? [])].sort((a, b) => a.order - b.order))
const polygons = computed(() => lines.value.map(line => ({ id: line.id, text: line.text,
  points: ocrPolygon(line.polygon, props.frame.width, props.frame.height) })).filter(line => line.points))
async function run(task: () => Promise<void>) {
  if (busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function recognize() {
  await run(async () => {
    pendingLink.value = null
    if (props.mode !== 'qr') {
      ocr.value = await api.recognizePinText()
      selected.value = ocr.value.lines.map(line => line.id)
    } else qr.value = await api.recognizePinQr()
  })
}
function toggle(id: string) {
  selected.value = selected.value.includes(id) ? selected.value.filter(value => value !== id) : [...selected.value, id]
  message.value = ''
}
async function copy() {
  // 原生复制使用明确的文本；用户在结果中选中的部分优先于勾选的整行。
  const area = textArea.value
  const value = area && area.selectionStart !== area.selectionEnd
    ? text.value.slice(area.selectionStart, area.selectionEnd) : text.value
  await run(async () => { await api.copyOcrText(value); message.value = '文字已复制' })
}
async function qrAction(index: number, action: 'open' | 'copy') {
  const result = qr.value
  if (!result) return
  await run(async () => {
    await api.pinQrAction(result.id, index, action)
    pendingLink.value = null
    message.value = action === 'copy' ? '已复制' : '已请求浏览器打开链接'
  })
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); if (!busy.value) emit('close') }
  // 对话框内循环焦点，防止 Tab 移到背后的贴图工具。
  if (event.key !== 'Tab') return
  const nodes = [...(panel.value?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled)') ?? [])]
  if (!nodes.length) { event.preventDefault(); return }
  const first = nodes[0], last = nodes[nodes.length - 1]
  if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.value)) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.value)) { event.preventDefault(); first?.focus() }
}
onMounted(() => { void nextTick(() => panel.value?.focus()); void recognize() })
</script>

<template>
  <div class="pin-recognition-shade" @pointerdown.stop @wheel.stop @contextmenu.stop.prevent>
    <section v-if="translating && ocr" class="pin-recognition" role="dialog" aria-modal="true" :aria-label='t("图片翻译")'>
      <ImageTranslation :recognition="ocr" :image="frame" @close="emit('close')" @back="backToRecognition" />
    </section>
    <section v-else ref="panel" class="pin-recognition" role="dialog" aria-modal="true" :aria-label="mode !== 'qr' ? t('图片文字识别') : t('图片二维码识别')" tabindex="-1" @keydown.stop="key">
      <header><strong>{{ mode !== 'qr' ? t("图片文字识别") : t("图片二维码识别") }}</strong><button :disabled="busy" @click="emit('close')">{{ t("关闭") }}</button></header>
      <p v-if="busy" role="status">{{ t("正在处理…") }}</p>
      <p v-if="error" role="alert" class="recognition-error">{{ te(error) }}</p>
      <p v-if="message" role="status">{{ t(message) }}</p>
      <template v-if="mode !== 'qr' && ocr">
        <p v-if="!lines.length">{{ t("未识别到文字。") }}</p>
        <p v-for="warning in ocr.warnings" :key="warning">{{ t(warning) }}</p>
        <div class="image-preview">
          <img :src="frame.dataUrl" :alt='t("识别原图（不含标注）")' draggable="false" @error="error = '预览加载失败，可从文字列表复制。'" />
          <svg :viewBox="`0 0 ${frame.width} ${frame.height}`" preserveAspectRatio="none" aria-hidden="true">
            <polygon v-for="line in polygons" :key="line.id" :points="line.points" :class="{ selected: selected.includes(line.id) }" @click="!busy && toggle(line.id)"><title>{{ line.text }}</title></polygon>
          </svg>
        </div>
        <div v-if="lines.length" class="line-list" :aria-label='t("选择识别文字行")'>
          <label v-for="line in lines" :key="line.id"><input v-model="selected" type="checkbox" :value="line.id" :disabled="busy" />{{ line.text }}</label>
        </div>
        <textarea ref="textArea" :value="text" readonly :aria-label='t("所选文字，可选择部分复制")' :placeholder='t("请选择文字行")' />
        <footer><button :disabled="busy || !lines.length" @click="selected = lines.map(line => line.id)">{{ t("全选") }}</button><button :disabled="busy || !selected.length" @click="selected = []">{{ t("清空选择") }}</button><button :disabled="busy || !text.trim()" @click="copy">{{ t("复制文字 / 所选片段") }}</button></footer>
      </template>
      <template v-if="mode === 'qr' && qr">
        <p v-if="!qr.items.length">{{ t("未发现二维码，请保留完整码及白边。") }}</p>
        <div v-for="(item, index) in qr.items" :key="index" class="qr-item">
          <p class="qr-text" dir="auto">{{ item.text }}</p>
          <footer><button :disabled="busy" @click="qrAction(index, 'copy')">{{ t("复制内容") }}</button><button v-if="item.url" :disabled="busy" @click="pendingLink = index">{{ t("打开链接…") }}</button></footer>
          <div v-if="pendingLink === index" role="group" :aria-label='t("确认打开链接")'>
            <p>{{ t("将在浏览器打开：") }}<span class="qr-text">{{ item.url }}</span></p>
            <footer><button :disabled="busy" @click="pendingLink = null">{{ t("取消") }}</button><button :disabled="busy" @click="qrAction(index, 'open')">{{ t("确认打开") }}</button></footer>
          </div>
        </div>
      </template>
      <footer><button :disabled="busy" @click="recognize">{{ t("重新识别") }}</button><button v-if="ocr && mode !== 'qr'" :disabled="busy || !ocr.fullText.trim()" @click="translating = true">{{ t("翻译图片") }}</button></footer>
    </section>
  </div>
</template>

<style scoped>
.pin-recognition-shade { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; background: #0003; padding: 8px; }
.pin-recognition { width: min(640px, 100%); max-height: 100%; overflow: auto; padding: 12px; background: #fff; border: 1px solid #d8dee4; border-radius: 6px; color: #24292f; font-size: 13px; user-select: text; }
header, footer { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
header { justify-content: space-between; border-bottom: 1px solid #e1e4e8; padding-bottom: 8px; }
footer { margin-top: 8px; }
button { min-height: 32px; padding: 4px 8px; border: 1px solid #d8dee4; border-radius: 4px; }
button:disabled { opacity: .5; }
button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.image-preview { position: relative; margin: 8px 0; }
.image-preview img { display: block; width: 100%; height: auto; }
svg { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
polygon { fill: #2563eb0a; stroke: #2563eb; stroke-width: 1; vector-effect: non-scaling-stroke; pointer-events: auto; cursor: pointer; }
polygon.selected { fill: #2563eb40; }
.line-list { max-height: 180px; overflow: auto; }
.line-list label { display: flex; align-items: start; gap: 8px; padding: 4px 0; overflow-wrap: anywhere; }
textarea { display: block; width: 100%; min-height: 100px; margin-top: 8px; padding: 8px; border: 1px solid #d8dee4; resize: vertical; }
.recognition-error { color: #b91c1c; overflow-wrap: anywhere; }
.qr-text { white-space: pre-wrap; overflow-wrap: anywhere; }
.qr-item { border-bottom: 1px solid #e1e4e8; padding: 8px 0; }
</style>
