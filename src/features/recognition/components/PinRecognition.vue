<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import ImageTranslation from '@/features/recognition/components/ImageTranslation.vue'
import OcrResultCard from '@/features/recognition/components/OcrResultCard.vue'
import { api } from '@/services/api'
import type { PinFrame } from '@/features/pins/types'
import type { QrResult } from '@/features/recognition/types'
import type { OcrResult } from '@/features/recognition/ai'

const props = defineProps<{ frame: PinFrame; mode: 'ocr' | 'qr' | 'translate' }>()
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)
const busy = ref(false), error = ref(''), message = ref('')
let disposed = false
const ocr = ref<OcrResult | null>(null), qr = ref<QrResult | null>(null)
// 与截图一致：识别结果是可编辑草稿，翻译来回切换不会丢失修正。
const draft = ref('')
const translating = ref(props.mode === 'translate')
function backToRecognition() { translating.value = false }
const pendingLink = ref<number | null>(null)
async function run(task: () => Promise<void>) {
  if (busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await task() } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function recognize() {
  await run(async () => {
    pendingLink.value = null
    if (props.mode !== 'qr') {
      const result = await api.recognizePinText()
      if (!disposed) { ocr.value = result; draft.value = result.fullText }
    } else {
      const result = await api.recognizePinQr()
      if (!disposed) qr.value = result
    }
  })
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
  // 识别处理中也允许返回贴图；迟到结果由 disposed 忽略，不重新打开面板。
  if (handleEscape(event, () => emit('close'))) return
  // 对话框内循环焦点，防止 Tab 移到背后的贴图工具。
  if (event.key !== 'Tab') return
  const nodes = [...(panel.value?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled)') ?? [])]
  if (!nodes.length) { event.preventDefault(); return }
  const first = nodes[0], last = nodes[nodes.length - 1]
  if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.value)) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.value)) { event.preventDefault(); first?.focus() }
}
onMounted(() => { void nextTick(() => panel.value?.focus()); void recognize() })
onUnmounted(() => { disposed = true })
</script>

<template>
  <div class="pin-recognition-shade" @pointerdown.stop @wheel.stop @contextmenu.stop.prevent>
    <section v-if="mode !== 'qr' && ocr" class="pin-recognition result-shell" role="dialog" aria-modal="true" :aria-label="translating ? t('文字翻译') : t('文字识别结果')">
      <ImageTranslation v-if="translating" :recognition="ocr" :source-text="draft" :image="frame" @close="emit('close')" @back="backToRecognition" />
      <OcrResultCard v-else v-model:text="draft" @translate="translating = true" @close="emit('close')" />
    </section>
    <section v-else ref="panel" class="pin-recognition" role="dialog" aria-modal="true" :aria-label="mode !== 'qr' ? t('图片文字识别') : t('图片二维码识别')" tabindex="-1" @keydown.stop="key">
      <header><strong>{{ mode !== 'qr' ? t("图片文字识别") : t("图片二维码识别") }}</strong><button @click="emit('close')">{{ t("关闭") }}</button></header>
      <div class="pin-recognition-body">
        <p v-if="busy" role="status">{{ t("正在处理…") }}</p>
        <p v-if="error" role="alert" class="recognition-error">{{ te(error) }}</p>
        <p v-if="message" role="status">{{ t(message) }}</p>
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
      </div>
      <footer><button :disabled="busy" @click="recognize">{{ t("重新识别") }}</button></footer>
    </section>
  </div>
</template>

<style scoped>
.pin-recognition-shade { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; background: #0003; padding: 8px; }
.pin-recognition { width: min(1280px, calc(100vw - 16px), calc((100vh - 16px) / 0.618)); aspect-ratio: 1 / 0.618; max-height: 100%; display: flex; flex-direction: column; min-height: 0; overflow: hidden; padding: 12px; background: #fff; border: 1px solid #d8dee4; border-radius: 8px; color: #24292f; font-size: 13px; user-select: text; }
.pin-recognition-body { flex: 1; min-height: 0; overflow: auto; }
.result-shell { padding: 0; border: 0; }
.result-shell :deep(.recognition-result) { flex: 1; min-height: 0; }
header, footer { flex-shrink: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
header { justify-content: space-between; border-bottom: 1px solid #e1e4e8; padding-bottom: 8px; }
footer { margin-top: 8px; }
button { min-height: 32px; padding: 4px 8px; border: 1px solid #d8dee4; border-radius: 4px; }
button:disabled { opacity: .5; }
button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.recognition-error { color: #b91c1c; overflow-wrap: anywhere; }
.qr-text { white-space: pre-wrap; overflow-wrap: anywhere; }
.qr-item { border-bottom: 1px solid #e1e4e8; padding: 8px 0; }
</style>
