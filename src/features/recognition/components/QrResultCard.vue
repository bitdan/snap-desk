<script setup lang="ts">
import { t } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import type { QrResult } from '@/features/recognition/types'
import Button from '@/shared/ui/button/Button.vue'

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
    <header><strong>{{ result.items.length ? t("发现二维码") : t("未发现二维码") }}</strong><Button variant="ghost" @click="close">{{ t("关闭") }}</Button></header>
    <p v-if="!result.items.length">{{ t("请完整框选二维码并保留周围白边，然后重试。") }}</p>
    <div class="qr-items">
      <div v-for="(item, index) in result.items" :key="index" class="qr-item">
        <p dir="auto">{{ item.text }}</p>
        <div class="qr-actions">
          <Button v-if="item.url" :disabled="busy" @click="act(index, 'open')">{{ t("打开") }}</Button>
          <Button variant="outline" :disabled="busy" @click="act(index, 'copy')">{{ item.url ? t("复制链接") : t("复制文本") }}</Button>
        </div>
      </div>
    </div>
    <p v-if="message" role="status">{{ t(message) }}</p>
  </section>
  <p v-else-if="message && !dismissed" class="notice error" role="alert">{{ t(message) }}</p>
</template>

<style scoped>
.qr-result { margin-bottom: 16px; padding: 16px; border: 1px solid #d7dce5; border-radius: 12px; background: #fff; }
header, .qr-actions { display: flex; align-items: center; gap: 8px; }
header { justify-content: space-between; }
.qr-items { max-height: 300px; overflow-y: auto; }
.qr-item + .qr-item { border-top: 1px solid #e5e7eb; margin-top: 12px; }
p { white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; margin: 10px 0; }
</style>
