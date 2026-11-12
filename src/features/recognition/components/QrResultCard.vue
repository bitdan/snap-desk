<script setup lang="ts">
import { t } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import type { QrResult } from '@/features/recognition/types'
import QrResultView from '@/features/recognition/components/QrResultView.vue'

// 仅由截图窗口传入主动识别结果，不订阅主界面事件。
const props = defineProps<{ captureResult: QrResult }>()
const emit = defineEmits<{ close: []; opened: [] }>()
const card = ref<HTMLElement | null>(null)

const result = computed(() => props.captureResult)
const dismissed = ref(false)
const busy = ref(false)
const message = ref('')
watch(() => props.captureResult, () => {
  dismissed.value = false
  message.value = ''
})
onMounted(() => card.value?.focus())

function close() { dismissed.value = true; emit('close') }
function key(event: KeyboardEvent) { handleEscape(event, close) }

async function act(index: number, action: 'open' | 'copy') {
  if (!result.value || busy.value) return
  const id = result.value.id
  busy.value = true; message.value = ''
  try {
    await api.qrAction(id, index, action)
    if (action === 'open') emit('opened')
    if (result.value?.id === id) message.value = action === 'copy' ? '已复制' : '已请求浏览器打开链接'
  } catch (error) { if (result.value?.id === id) message.value = String(error) }
  finally { busy.value = false }
}
</script>

<template>
  <section v-if="!dismissed" ref="card" tabindex="-1" class="qr-result" :aria-label='t("二维码识别结果")' @keydown.stop="key">
    <QrResultView :result="result" :busy="busy" :message="message" @copy="act($event, 'copy')" @open="act($event, 'open')" @close="close" />
  </section>
</template>

<style scoped>
.qr-result { width: 100%; height: 100%; outline: none; }
</style>
