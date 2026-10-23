<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/lib/api'
import { targetLanguages, type AiSettings, type OcrResult } from '@/lib/ai'
import { imageTranslationPlan, overlayLayoutReason, overlayPosition, translateImageUnits, type TranslationImage, type ImageTextUnit } from '@/lib/image-translation'

const props = defineProps<{ recognition: OcrResult; image: TranslationImage; sourceText?: string }>()
const emit = defineEmits<{ close: []; back: [] }>()
const panel = ref<HTMLElement | null>(null), imageArea = ref<HTMLElement | null>(null)
const config = ref<AiSettings | null>(null), consent = ref(false)
const loading = ref(false), busy = ref(false), copying = ref(false)
const error = ref(''), message = ref(''), compare = ref(false)
const results = ref<Record<string, string>>({})
const hidden = ref<string[]>([]), positions = ref<Record<string, { x: number; y: number }>>({})
const plan = computed(() => imageTranslationPlan(props.recognition, props.sourceText))
const displaySize = ref({ width: 0, height: 0 })
const crop = computed(() => props.image.crop ?? { x: 0, y: 0, width: props.image.width, height: props.image.height })
const fallbackReason = computed(() => plan.value.reason || (crop.value.width !== props.recognition.imageWidth || crop.value.height !== props.recognition.imageHeight ? '原图与文字框尺寸不一致，显示文本对照。' : '') || overlayLayoutReason(plan.value, props.recognition.imageWidth, props.recognition.imageHeight, displaySize.value.width, displaySize.value.height, results.value))
const comparison = computed(() => compare.value || !!fallbackReason.value)
const local = computed(() => config.value?.provider === 'local-ctranslate2')
const target = computed(() => targetLanguages.find(item => item.value === config.value?.targetLanguage)?.label ?? '目标语言')
const completed = computed(() => plan.value.units.filter(unit => results.value[unit.id]).length)
const canTranslate = computed(() => !!config.value && !loading.value && !busy.value && !plan.value.error && plan.value.units.length > 0
  && (local.value || (consent.value && !!config.value.endpoint && !!config.value.model)))
const visible = computed(() => plan.value.units.filter(unit => unit.bounds && results.value[unit.id] && !hidden.value.includes(unit.id)))
let disposed = false, cancelled = false, requestId: string | null = null

/** 只读已保存配置，不在打开面板时上传文字；重新读取时撤销上次的发送确认。 */
async function loadConfig() {
  if (loading.value || busy.value) return
  loading.value = true; error.value = ''; consent.value = false; config.value = null
  results.value = {}; positions.value = {}; hidden.value = []; message.value = ''
  try {
    const value = await api.aiTranslationConfig()
    if (!disposed) {
      config.value = value
      if (value.provider === 'online-llm' && (!value.endpoint || !value.model)) error.value = '请先在主窗口的识别与翻译设置中配置在线服务。'
    }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) loading.value = false }
}
async function translate() {
  if (!canTranslate.value || !config.value) return
  busy.value = true; cancelled = false; error.value = ''; message.value = ''
  const settings = { ...config.value }
  const units = plan.value.units.filter(unit => !results.value[unit.id])
  try {
    await translateImageUnits(units, settings, consent.value, {
      translate: api.translateText, newId: () => crypto.randomUUID(), cancelled: () => cancelled || disposed,
      pending: id => { requestId = id },
      completed: (unit, result) => { if (!disposed) results.value[unit.id] = result.translatedText },
    })
    if (!disposed) message.value = '翻译已完成'
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function cancel() {
  cancelled = true
  const id = requestId
  if (id) {
    try { await api.cancelAiJob(id) }
    catch (e) { if (!disposed) error.value = `已停止后续翻译，当前请求取消失败：${String(e)}` }
  }
}
async function copy(text: string) {
  if (!text.trim() || copying.value) return
  copying.value = true; message.value = ''
  try { await api.copyOcrText(text); if (!disposed) message.value = '译文已复制' }
  catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) copying.value = false }
}
const allText = computed(() => plan.value.units.map(unit => results.value[unit.id]).filter(Boolean).join('\n'))
function position(unit: ImageTextUnit) {
  return positions.value[unit.id] ?? overlayPosition(unit.bounds!.x / props.recognition.imageWidth, unit.bounds!.y / props.recognition.imageHeight)
}
function overlayStyle(unit: ImageTextUnit) {
  const p = position(unit)
  return { left: `${p.x * 100}%`, top: `${p.y * 100}%`, width: `${Math.min(0.8, Math.max(0.2, unit.bounds!.width / props.recognition.imageWidth)) * 100}%`, maxWidth: `${(1 - p.x) * 100}%` }
}
let drag: { id: string; pointer: number; clientX: number; clientY: number; x: number; y: number; width: number; height: number } | null = null
/** 把指针的 CSS 位移换成图像比例；无需读取系统 DPI，也不修改原图。 */
function startMove(event: PointerEvent, unit: ImageTextUnit) {
  if (event.button !== 0 || !imageArea.value) return
  const bounds = imageArea.value.getBoundingClientRect(), p = position(unit)
  if (!bounds.width || !bounds.height) return
  event.preventDefault()
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  drag = { id: unit.id, pointer: event.pointerId, clientX: event.clientX, clientY: event.clientY, x: p.x, y: p.y, width: bounds.width, height: bounds.height }
}
function moving(event: PointerEvent) {
  if (!drag || drag.pointer !== event.pointerId) return
  positions.value[drag.id] = overlayPosition(drag.x + (event.clientX - drag.clientX) / drag.width, drag.y + (event.clientY - drag.clientY) / drag.height)
}
function moveKey(event: KeyboardEvent, unit: ImageTextUnit) {
  if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return
  event.preventDefault(); event.stopPropagation()
  const p = position(unit), step = event.shiftKey ? 0.05 : 0.01
  positions.value[unit.id] = overlayPosition(p.x + (event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0), p.y + (event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0))
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); emit('close'); return }
  if (event.key !== 'Tab') return
  const nodes = [...(panel.value?.querySelectorAll<HTMLElement>('textarea:not(:disabled), input:not(:disabled), button:not(:disabled)') ?? [])]
  const first = nodes[0], last = nodes[nodes.length - 1]
  if (!nodes.length) { event.preventDefault(); return }
  if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.value)) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.value)) { event.preventDefault(); first?.focus() }
}
let observer: ResizeObserver | null = null
onMounted(() => {
  void nextTick(() => {
    panel.value?.focus()
    observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect
      if (rect) displaySize.value = { width: rect.width, height: rect.height }
    })
    if (imageArea.value) observer.observe(imageArea.value)
  })
  void loadConfig()
})
onUnmounted(() => { disposed = true; observer?.disconnect(); void cancel() })
</script>

<template>
  <section ref="panel" class="image-translation" tabindex="-1" aria-label="图片翻译" @keydown.stop="key">
    <header><strong>图片翻译</strong><button @click="emit('close')">关闭</button></header>
    <div class="translation-actions">
      <span v-if="loading" role="status">正在读取配置…</span>
      <template v-else-if="config"><span>{{ local ? '本地离线 · 图片与文字不上传' : '在线翻译 · 仅发送文字' }} → {{ target }}</span><span>{{ completed }} / {{ plan.units.length }}</span></template>
      <button :disabled="loading || busy" @click="loadConfig">重新读取配置</button>
    </div>
    <template v-if="config && !local">
      <p class="destination">{{ config.endpoint }} · {{ config.model }}</p>
      <label class="consent"><input v-model="consent" type="checkbox" :disabled="busy" />同意将本次识别文字发送至上述服务（{{ plan.units.length }} 个文本片段，可能计费）</label>
    </template>
    <p v-if="fallbackReason" role="status">{{ fallbackReason }}</p>
    <p v-if="plan.error || error" class="translation-error" role="alert">{{ plan.error || error }}</p>
    <p v-if="message" class="translation-success" role="status">{{ message }}</p>
    <div class="translation-actions">
      <button v-if="busy" @click="cancel">取消翻译</button>
      <button :disabled="!canTranslate || completed === plan.units.length" @click="translate">{{ completed ? '继续翻译未完成部分' : '翻译图片' }}</button>
      <button :disabled="!allText || copying" @click="copy(allText)">复制已有译文</button>
      <label v-if="!fallbackReason"><input v-model="compare" type="checkbox" />文本对照视图</label>
      <button v-if="hidden.length && !comparison" @click="hidden = []">恢复已关闭译文</button>
    </div>
    <div ref="imageArea" class="translation-image">
      <svg :viewBox="`0 0 ${crop.width} ${crop.height}`" :style="{ aspectRatio: `${crop.width} / ${crop.height}` }" role="img" aria-label="原始图片，译文不会写入原图">
        <image :href="image.dataUrl" :x="-crop.x" :y="-crop.y" :width="image.width" :height="image.height" preserveAspectRatio="none" />
      </svg>
      <template v-if="!comparison">
        <section v-for="unit in visible" :key="unit.id" class="translation-overlay" :style="overlayStyle(unit)" :aria-label="`译文：${unit.source}`">
          <div class="overlay-actions"><button class="move-translation" :aria-label="`移动译文：${unit.source}，可用方向键`" @pointerdown="startMove($event, unit)" @pointermove="moving" @pointerup="drag = null" @pointercancel="drag = null" @lostpointercapture="drag = null" @keydown="moveKey($event, unit)">移动</button><button :disabled="copying" @click="copy(results[unit.id])">复制</button><button @click="hidden.push(unit.id)">关闭</button></div>
          <p class="overlay-source">{{ unit.source }}</p><p class="overlay-text" dir="auto">{{ results[unit.id] }}</p>
        </section>
      </template>
    </div>
    <div v-if="comparison" class="comparison-list" aria-label="原文译文对照">
      <article v-for="unit in plan.units" :key="unit.id"><div><strong>原文</strong><p dir="auto">{{ unit.source }}</p></div><div><strong>译文</strong><p dir="auto">{{ results[unit.id] || '尚未翻译' }}</p><button v-if="results[unit.id]" :disabled="copying" @click="copy(results[unit.id])">复制译文</button></div></article>
    </div>
    <label class="original-text">原始识别文字<textarea :value="recognition.fullText" readonly aria-label="原始识别文字" /></label>
    <p v-for="warning in recognition.warnings" :key="warning">{{ warning }}</p>
    <footer><button @click="emit('back')">返回识别</button></footer>
  </section>
</template>

<style scoped>
.image-translation { color: #24292f; font-size: 13px; padding: 12px; background: #fff; user-select: text; }
header, .translation-actions, .overlay-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
header { justify-content: space-between; border-bottom: 1px solid #e1e4e8; padding-bottom: 8px; }
.translation-actions, footer { margin: 8px 0; }
button { min-height: 32px; padding: 4px 8px; border: 1px solid #d8dee4; border-radius: 4px; background: #fff; }
button:disabled { opacity: .5; }
button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.destination, .consent, .translation-error { overflow-wrap: anywhere; }
.consent { display: flex; align-items: start; gap: 8px; }
.translation-error { color: #b91c1c; }
.translation-success { color: #24834b; }
.translation-image { position: relative; margin-top: 12px; overflow: hidden; }
.translation-image > svg { display: block; width: 100%; height: auto; }
.translation-overlay { position: absolute; border: 1px solid #2563eb; border-radius: 4px; background: #ffffffed; box-shadow: 0 1px 4px #0002; max-height: 200px; overflow: auto; }
.overlay-actions { gap: 2px; position: sticky; top: 0; background: #f6f7f8; }
.overlay-actions button { padding: 2px 4px; min-height: 28px; font-size: 12px; }
.move-translation { touch-action: none; cursor: move; }
.overlay-source { font-size: 12px; color: #57606a; margin: 4px; }
.overlay-text { margin: 4px; }
p { white-space: pre-wrap; overflow-wrap: anywhere; }
.comparison-list article { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; padding: 12px 0; border-bottom: 1px solid #e1e4e8; }
.original-text { display: block; margin-top: 12px; }
textarea { display: block; width: 100%; height: 100px; padding: 8px; margin-top: 4px; border: 1px solid #d8dee4; resize: vertical; }
@media (max-width: 500px) { .comparison-list article { grid-template-columns: minmax(0, 1fr); } }
</style>
