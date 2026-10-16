<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/lib/api'
import type { AiSettings, OcrResult } from '@/lib/ai'
import Button from './ui/button/Button.vue'

const props = defineProps<{ result: OcrResult }>()
const emit = defineEmits<{ close: [] }>()
const source = ref(props.result.fullText), translated = ref(''), consent = ref(false)
const busy = ref(false), error = ref(''), message = ref(''), target = ref('')
const input = ref<HTMLTextAreaElement | null>(null)
const config = ref<AiSettings | null>(null)
const local = computed(() => config.value?.provider === 'local-ctranslate2')
const canTranslate = computed(() => !!config.value && (local.value || (consent.value && !!config.value.endpoint && !!config.value.model)))
let requestId: string | null = null, disposed = false
onMounted(async () => {
  void nextTick(() => input.value?.focus())
  try {
    config.value = await api.aiTranslationConfig()
    if (config.value.provider === 'online-llm' && (!config.value.endpoint || !config.value.model)) error.value = '请先在主窗口的识别与翻译页配置服务地址、模型与密钥。'
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; if (requestId) void api.cancelAiJob(requestId).catch(() => {}) })
async function translate() {
  if (busy.value || !canTranslate.value || !source.value.trim() || !config.value) return
  const id = crypto.randomUUID(); requestId = id
  busy.value = true; error.value = ''; message.value = ''; translated.value = ''
  try {
    const result = await api.translateText(id, source.value, consent.value, config.value)
    if (!disposed && requestId === id && result.requestId === id) {
      translated.value = result.translatedText; target.value = result.targetLanguage
    }
  } catch (e) { if (!disposed && requestId === id) error.value = String(e) }
  finally { if (requestId === id) { busy.value = false; requestId = null } }
}
async function cancel() {
  if (!requestId) return
  try { await api.cancelAiJob(requestId) }
  catch (e) { error.value = String(e) }
}
async function copy() {
  try { await api.copyOcrText(translated.value); message.value = '译文已复制。' }
  catch (e) { error.value = String(e) }
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); emit('close') }
  if (event.key === 'Tab') {
    const nodes = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('textarea:not(:disabled), input:not(:disabled), button:not(:disabled)')]
    const first = nodes[0], last = nodes[nodes.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
  }
}
</script>

<template>
  <section class="translation-panel" @keydown.stop="key">
    <header><strong>截图翻译</strong><Button variant="ghost" @click="emit('close')">关闭</Button></header>
    <p class="hint">{{ local ? '本地离线翻译，图片与文字均不上传。可修正原文后重新翻译。' : '可修正原文后发送到翻译服务；仅发送文字，不上传图片。' }}</p>
    <p v-if="!local && config?.endpoint" class="hint destination">服务：{{ config.endpoint }} · 模型：{{ config.model }} · 目标：{{ config.targetLanguage }}</p>
    <p v-if="local" class="hint">{{ config?.targetLanguage === 'en' ? '中文 → 英语' : '英语 → 简体中文' }} · CTranslate2 / OPUS-MT</p>
    <div class="columns">
      <label>原文<textarea ref="input" v-model="source" :disabled="busy" aria-label="原文，可编辑" @input="translated = ''; message = ''" /></label>
      <label>译文{{ target ? `（${target}）` : '' }}<textarea v-model="translated" readonly aria-label="译文" /></label>
    </div>
    <label v-if="!local" class="consent"><input v-model="consent" type="checkbox" :disabled="busy" />同意将以上原文发送给已配置的服务（可能计费）</label>
    <footer><Button :disabled="busy || !canTranslate || !source.trim()" @click="translate">{{ busy ? '翻译中…' : '翻译 / 重新翻译' }}</Button><Button v-if="busy" variant="outline" @click="cancel">取消</Button><Button variant="outline" :disabled="!translated" @click="copy">复制译文</Button></footer>
    <p v-if="!source.trim()" class="hint">未识别到文字，可返回重新框选。</p>
    <p v-if="message" role="status">{{ message }}</p><p v-if="error" class="error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.translation-panel { background: white; padding: 16px; border: 1px solid #e1e4e8; border-radius: 6px; font-size: 13px; max-height: 85vh; overflow: auto; }
.destination { overflow-wrap: anywhere; }
header, footer { display: flex; align-items: center; gap: 8px; } header { justify-content: space-between; } footer { margin-top: 12px; flex-wrap: wrap; }
.columns { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }.columns label { min-width: 0; }
textarea { display: block; width: 100%; height: min(280px, 32vh); margin-top: 8px; padding: 8px; border: 1px solid #d8dee4; border-radius: 4px; resize: vertical; user-select: text; }
textarea:focus-visible, input:focus-visible { outline: 2px solid #2563eb; }
.hint { font-size: 12px; color: #57606a; margin: 8px 0; }.consent { display: flex; align-items: center; gap: 8px; margin-top: 12px; }.error { color: #b91c1c; margin-top: 8px; }
@media (max-width: 600px) { .columns { grid-template-columns: 1fr; } textarea { height: 120px; } }
</style>
