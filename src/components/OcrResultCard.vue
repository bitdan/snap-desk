<script setup lang="ts">
import { nextTick, onMounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import { api } from '@/lib/api'

const props = defineProps<{ text: string }>()
const emit = defineEmits<{ close: [] }>()
const text = ref(props.text), busy = ref(false), message = ref(''), error = ref('')
const input = ref<HTMLTextAreaElement | null>(null)
onMounted(() => { void nextTick(() => input.value?.focus()) })
async function copy() {
  if (busy.value || !text.value.trim()) return
  busy.value = true; message.value = ''; error.value = ''
  try { await api.copyOcrText(text.value); message.value = '文字已复制。' }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape' && !busy.value) { event.preventDefault(); emit('close') }
  // 焦点限制在结果面板中，避免 Tab 移到截图工具栏后意外保存或关闭会话。
  if (event.key === 'Tab') {
    const nodes = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('textarea, button:not(:disabled)')]
    const first = nodes[0], last = nodes[nodes.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
  }
}
</script>

<template>
  <section class="ocr-result" @keydown.stop="key">
    <header><strong>文字识别结果</strong><Button variant="ghost" :disabled="busy" @click="emit('close')">关闭</Button></header>
    <p v-if="!props.text" class="text-xs text-zinc-500">未识别到文字，请框选清晰、正向的文字后重试。</p>
    <textarea ref="input" v-model="text" aria-label="识别文字，可编辑" :disabled="busy" @input="message = ''" />
    <footer><Button :disabled="busy || !text.trim()" @click="copy">复制文字</Button><span role="status">{{ message }}</span></footer>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.ocr-result { background: white; padding: 16px; border: 1px solid #e1e4e8; border-radius: 6px; font-size: 13px; }
header, footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
textarea { display: block; width: 100%; height: min(320px, 45vh); resize: vertical; margin: 12px 0; padding: 8px; border: 1px solid #d8dee4; border-radius: 4px; user-select: text; }
textarea:focus-visible { outline: 2px solid #2563eb; }
</style>
