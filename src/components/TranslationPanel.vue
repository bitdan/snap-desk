<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/lib/api'
import { targetLanguages, type AiSettings } from '@/lib/ai'
import { ArrowLeft, Check, Copy, Languages, LoaderCircle, X } from 'lucide-vue-next'
import Button from './ui/button/Button.vue'
import './recognition-results.css'

const emit = defineEmits<{ close: []; back: [] }>()
const source = defineModel<string>('text', { required: true })
const translated = ref(''), consent = ref(false)
const busy = ref(false), error = ref(''), message = ref(''), target = ref('')
const loading = ref(false), copying = ref(false)
const input = ref<HTMLTextAreaElement | null>(null)
const config = ref<AiSettings | null>(null)
const local = computed(() => config.value?.provider === 'local-ctranslate2')
const canTranslate = computed(() => !!config.value && (local.value || (consent.value && !!config.value.endpoint && !!config.value.model)))
const targetLabel = computed(() => targetLanguages.find(item => item.value === (target.value || config.value?.targetLanguage))?.label ?? '目标语言')
const sourceLabel = computed(() => local.value ? (config.value?.targetLanguage === 'en' ? '简体中文' : '英语') : '原文')
const count = computed(() => Array.from(source.value).length)
let requestId: string | null = null, disposed = false
/** 仅加载已保存配置；进入翻译面板不会自动发送原文。 */
async function loadConfig() {
  if (loading.value) return
  loading.value = true; error.value = ''
  try {
    const value = await api.aiTranslationConfig()
    if (disposed) return
    config.value = value
    if (config.value.provider === 'online-llm' && (!config.value.endpoint || !config.value.model)) error.value = '请先在“识别与翻译”中配置在线服务。'
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { loading.value = false }
}
onMounted(() => { void nextTick(() => input.value?.focus()); void loadConfig() })
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
  if (copying.value || !translated.value) return
  copying.value = true; message.value = ''; error.value = ''
  try { await api.copyOcrText(translated.value); message.value = '译文已复制。' }
  catch (e) { error.value = String(e) }
  finally { copying.value = false }
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
  <section class="recognition-result translation-result" @keydown.stop="key">
    <header class="result-header"><h2><Languages :size="17" />{{ t("文字翻译") }}</h2><Button class="result-close" variant="ghost" :title='t("关闭（Esc）")' :aria-label='t("关闭翻译")' @click="emit('close')"><X :size="16" /></Button></header>
    <div class="result-body">
      <div class="result-context"><span v-if="loading" role="status">{{ t("正在读取翻译配置…") }}</span><template v-else-if="config"><span class="result-badge">{{ local ? t("本地离线") : t("在线翻译") }}</span><span>{{ local ? t("文字与图片不上传") : t("仅发送文字，不上传图片") }}</span></template><Button v-else variant="outline" @click="loadConfig">{{ t("重新读取配置") }}</Button></div>
      <div class="result-columns">
        <div class="result-editor"><div class="editor-heading"><label for="translation-source">{{ t(sourceLabel) }}<span v-if="local" class="editor-subtitle">{{ t("原文") }}</span></label><span>{{ t("可编辑 ·") }} {{ count }} {{ t("字符") }}</span></div><textarea id="translation-source" ref="input" v-model="source" :disabled="busy" :aria-label='t("原文，可编辑")' :placeholder='t("输入或修正需要翻译的文字")' spellcheck="false" @input="translated = ''; target = ''; message = ''; error = ''" /></div>
        <div class="result-editor translated-editor" :aria-busy="busy"><div class="editor-heading"><label for="translation-output">{{ t(targetLabel) }}<span class="editor-subtitle">{{ t("译文") }}</span></label><span v-if="busy" class="result-progress" role="status"><LoaderCircle :size="13" class="result-spinner" />{{ t("翻译中") }}</span><span v-else-if="translated" class="result-success" role="status"><Check :size="13" />{{ t("已完成") }}</span></div><textarea id="translation-output" :value="translated" readonly :aria-label='t("译文")' :placeholder="busy ? t('正在翻译，请稍候…') : t('点击下方翻译，译文将在这里显示')" /></div>
      </div>
      <template v-if="config && !local"><p class="result-destination">{{ config.endpoint }} · {{ config.model }}</p><label class="result-consent"><input v-model="consent" type="checkbox" :disabled="busy" />{{ t("同意将原文发送给上述服务（可能计费）") }}</label></template>
      <p v-if="error" class="result-error" role="alert">{{ te(error) }}</p>
    </div>
    <footer class="result-footer"><Button variant="ghost" :disabled="copying" @click="emit('back')"><ArrowLeft :size="15" />{{ t("返回识别") }}</Button><span v-if="message" class="result-success" role="status"><Check :size="14" />{{ t(message) }}</span><div class="result-actions"><Button v-if="busy" variant="outline" @click="cancel">{{ t("取消翻译") }}</Button><Button variant="outline" :disabled="!translated || copying" @click="copy"><Copy :size="15" />{{ t("复制译文") }}</Button><Button :disabled="busy || copying || !canTranslate || !source.trim()" @click="translate"><LoaderCircle v-if="busy" :size="15" class="result-spinner" /><Languages v-else :size="15" />{{ busy ? t("翻译中…") : translated ? t("重新翻译") : t("翻译") }}</Button></div></footer>
  </section>
</template>
