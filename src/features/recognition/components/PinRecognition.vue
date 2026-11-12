<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import ConfirmDialog from '@/shared/ui/ConfirmDialog.vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { LoaderCircle, RotateCcw, ScanText } from 'lucide-vue-next'
import OcrWorkspace from '@/features/recognition/components/OcrWorkspace.vue'
import QrResultView from '@/features/recognition/components/QrResultView.vue'
import { initialOcrText, ocrWindowSize } from '@/features/recognition/ocr-workspace'
import { api } from '@/services/api'
import type { PinImage } from '@/features/pins/types'
import type { QrResult } from '@/features/recognition/types'
import type { OcrResult } from '@/features/recognition/ai'

const props = defineProps<{ frame: PinImage; mode: 'ocr' | 'qr' | 'translate' }>()
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)
const busy = ref(false), error = ref(''), message = ref('')
let disposed = false
const ocr = ref<OcrResult | null>(null), qr = ref<QrResult | null>(null)
// 与截图一致：识别结果是可编辑草稿，识别与翻译视图来回切换不会丢失修正。
const draft = ref('')
// 识别工作区左侧显示贴图原图；旋转等变换后宽高比不符时由工作区自动不画识别框。
const image = computed(() => ({ url: props.frame.src, width: props.frame.width, height: props.frame.height }))
const ocrWindow = computed(() => ocrWindowSize(image.value, globalThis.devicePixelRatio ?? 1, { width: globalThis.screen?.availWidth ?? 1280, height: globalThis.screen?.availHeight ?? 800 }))
const title = computed(() => props.mode === 'qr' ? t('图片二维码识别') : props.mode === 'translate' ? t('图片翻译') : t('文字识别结果'))
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
      if (!disposed) { draft.value = initialOcrText(result); ocr.value = result }
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
function updateLinkOpen(open: boolean) { if (!open && !busy.value) pendingLink.value = null }
function requestLink(index: number) { error.value = ''; pendingLink.value = index }
function key(event: KeyboardEvent) {
  if (pendingLink.value !== null) {
    handleEscape(event, () => { if (!busy.value) pendingLink.value = null })
    return
  }
  // 识别处理中也允许返回贴图；迟到结果由 disposed 忽略，不重新打开面板。
  handleEscape(event, () => emit('close'))
}
onMounted(() => { void nextTick(() => panel.value?.focus()); void recognize() })
onUnmounted(() => { disposed = true })
</script>

<template>
  <DetachedWindow :open="true" :title="title" :width="mode === 'qr' ? 480 : ocrWindow.width" :height="mode === 'qr' ? 420 : ocrWindow.height" @close="emit('close')">
    <OcrWorkspace v-if="mode !== 'qr' && ocr" v-model:text="draft" :result="ocr" :image="image" :mode="mode === 'translate' ? 'translate' : 'ocr'" :error="error" @close="emit('close')" />
    <section v-else-if="mode === 'qr'" ref="panel" class="pin-recognition" :aria-label="title" tabindex="-1" @keydown.stop="key">
      <QrResultView :result="qr" :loading="busy" :busy="busy" :error="error" :message="message" open-label="打开链接…" retry @copy="qrAction($event, 'copy')" @open="requestLink" @retry="recognize" @close="emit('close')" />
    </section>
    <section v-else ref="panel" class="pin-recognition" :aria-label="title" tabindex="-1" @keydown.stop="key">
      <div class="pin-recognition-body">
        <div v-if="busy" class="pin-recognition-state" role="status"><LoaderCircle :size="22" class="pin-recognition-spinner" aria-hidden="true" />{{ t('正在识别文字…') }}</div>
        <div v-else-if="error" class="pin-recognition-state">
          <ScanText :size="22" aria-hidden="true" />
          <p role="alert" class="pin-recognition-error">{{ te(error) }}</p>
        </div>
      </div>
      <footer class="pin-recognition-footer">
        <Button variant="ghost" :disabled="busy" @click="recognize"><RotateCcw :size="14" />{{ t("重新识别") }}</Button>
        <Button variant="outline" @click="emit('close')">{{ t("关闭") }}</Button>
      </footer>
    </section>
    <ConfirmDialog :open="pendingLink !== null" :title="t('确认打开链接')" :description="t('将在浏览器打开：') + '\n' + (qr?.items[pendingLink ?? -1]?.url ?? '')" :confirm-label="t('确认打开')" :busy="busy" :error="te(error)" @update:open="updateLinkOpen" @confirm="pendingLink !== null && qrAction(pendingLink, 'open')" />
  </DetachedWindow>
</template>

<style>
.pin-recognition { --pr-bg:#fff; --pr-soft:#f3f5f8; --pr-border:#e1e4e8; --pr-text:#24292f; --pr-muted:#6b7280; --pr-danger:#b42318;
  display:flex; flex-direction:column; width:100%; height:100%; min-height:0; background:var(--pr-bg); color:var(--pr-text); font-size:13px; user-select:text; outline:none; }
[data-theme=dark] .pin-recognition { --pr-bg:#1b1f24; --pr-soft:#252b33; --pr-border:#39424e; --pr-text:#e6edf3; --pr-muted:#9aa7b4; --pr-danger:#ff8a80; }
.pin-recognition-body { display:flex; flex:1; flex-direction:column; gap:8px; min-height:0; padding:12px; overflow:auto; }
.pin-recognition-state { display:flex; flex:1; flex-direction:column; align-items:center; justify-content:center; gap:10px; margin:0; color:var(--pr-muted); text-align:center; }
.pin-recognition-spinner { animation:result-spin 1s linear infinite; }
.pin-recognition-error { margin:0; color:var(--pr-danger); overflow-wrap:anywhere; }
.pin-recognition-footer { display:flex; align-items:center; justify-content:flex-end; gap:8px; padding:8px 12px; border-top:1px solid var(--pr-border); }
.pin-recognition-footer button { height:32px; padding:0 12px; }
@keyframes result-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .pin-recognition-spinner { animation:none; } }
</style>
