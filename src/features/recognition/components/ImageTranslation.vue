<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/services/api'
import { onlineTranslationReady, targetLanguages, type AiSettings, type OcrResult } from '@/features/recognition/ai'
import { imageTranslationPlan, translateImageUnits, type TranslationImage } from '@/features/recognition/image-translation'
import { ArrowLeft, Check, Copy, Languages, LoaderCircle, X } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import '@/features/recognition/components/recognition-results.css'

const props = defineProps<{ recognition: OcrResult; image: TranslationImage; sourceText?: string }>()
const emit = defineEmits<{ close: []; back: [] }>()
const panel = ref<HTMLElement | null>(null)
const config = ref<AiSettings | null>(null), consent = ref(false)
const loading = ref(false), busy = ref(false), copying = ref(false)
const error = ref(''), message = ref('')
const results = ref<Record<string, string>>({})
const plan = computed(() => imageTranslationPlan(props.recognition, props.sourceText))
const local = computed(() => config.value?.provider === 'local-ctranslate2')
const target = computed(() => targetLanguages.find(item => item.value === config.value?.targetLanguage)?.label ?? '目标语言')
const completed = computed(() => plan.value.units.filter(unit => results.value[unit.id]).length)
const canTranslate = computed(() => !!config.value && !loading.value && !busy.value && !plan.value.error && plan.value.units.length > 0
  && (local.value || (consent.value && onlineTranslationReady(config.value))))
let disposed = false, cancelled = false, requestId: string | null = null

/** 只读已保存配置，不在打开面板时上传文字；重新读取时撤销上次的发送确认。 */
async function loadConfig() {
  if (loading.value || busy.value) return
  loading.value = true; error.value = ''; consent.value = false; config.value = null
  results.value = {}; message.value = ''
  try {
    const value = await api.aiTranslationConfig()
    if (!disposed) {
      config.value = value
      if (value.provider !== 'local-ctranslate2' && !onlineTranslationReady(value)) error.value = '请先在“识别与翻译”中配置在线服务。'
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
      translate: api.translateText, translateBatch: api.translateTexts, newId: () => crypto.randomUUID(), cancelled: () => cancelled || disposed,
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
function key(event: KeyboardEvent) {
  if (handleEscape(event, () => emit('close'))) return
  if (event.key !== 'Tab') return
  const nodes = [...(panel.value?.querySelectorAll<HTMLElement>('textarea:not(:disabled), input:not(:disabled), button:not(:disabled)') ?? [])]
  const first = nodes[0], last = nodes[nodes.length - 1]
  if (!nodes.length) { event.preventDefault(); return }
  if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.value)) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.value)) { event.preventDefault(); first?.focus() }
}
// 面板只呈现原文和译文，退出时仍取消当前请求并停止后续片段。
onMounted(() => { void nextTick(() => panel.value?.focus()); void loadConfig() })
onUnmounted(() => { disposed = true; void cancel() })
</script>

<template>
  <section ref="panel" class="recognition-result translation-result image-translation" tabindex="-1" :aria-label='t("图片翻译")' @keydown.stop="key">
    <header class="result-header"><h2><Languages :size="17" />{{ t("图片翻译") }}</h2><Button class="result-close" variant="ghost" :title='t("关闭（Esc）")' :aria-label='t("关闭翻译")' @click="emit('close')"><X :size="16" /></Button></header>
    <div class="result-body">
      <div class="translation-stack">
        <section class="translation-text source-text" :aria-label='t("原文")'>
          <div class="text-heading"><span>{{ t("原文") }}</span><span v-if="loading" role="status">{{ t("正在读取配置…") }}</span></div>
          <div class="text-content" tabindex="0"><p v-for="unit in plan.units" :key="unit.id" dir="auto">{{ unit.source }}</p><p v-if="!plan.units.length">{{ t("未识别到文字。") }}</p></div>
        </section>
        <section class="translation-text translated-text" :aria-label='t("译文")' :aria-busy="busy">
          <div class="text-heading"><span>{{ t("译文") }}<template v-if="config"> · {{ t(target) }}</template></span><span v-if="busy" class="result-progress" role="status"><LoaderCircle :size="13" class="result-spinner" />{{ completed }} / {{ plan.units.length }}</span><span v-else-if="allText && completed === plan.units.length" class="result-success"><Check :size="13" />{{ t("已完成") }}</span></div>
          <div class="text-content" tabindex="0"><template v-for="unit in plan.units" :key="unit.id"><p v-if="results[unit.id]" dir="auto">{{ results[unit.id] }}</p></template><p v-if="!allText" class="text-placeholder">{{ busy ? t("正在翻译，请稍候…") : t("点击下方翻译，译文将在这里显示") }}</p></div>
        </section>
      </div>
      <template v-if="config && !local"><p class="result-destination">{{ config.endpoint }}<template v-if="config.provider === 'online-llm'"> · {{ config.model }}</template></p><label class="result-consent"><input v-model="consent" type="checkbox" :disabled="busy" />{{ t("同意将原文发送给上述服务（可能计费）") }}</label></template>
      <p v-if="error && config && !local" class="result-destination">{{ t("在线翻译失败，可在设置中选择本地离线翻译。") }}</p><p v-if="plan.error || error" class="result-error" role="alert">{{ te(plan.error || error) }}</p>
    </div>
    <footer class="result-footer"><Button variant="ghost" @click="emit('back')"><ArrowLeft :size="15" />{{ t("返回识别") }}</Button><span v-if="message" class="result-success" role="status"><Check :size="14" />{{ t(message) }}</span><div class="result-actions"><Button v-if="busy" variant="outline" @click="cancel">{{ t("取消翻译") }}</Button><Button variant="outline" :disabled="!allText || copying" @click="copy(allText)"><Copy :size="15" />{{ t("复制译文") }}</Button><Button :disabled="!canTranslate || completed === plan.units.length" @click="translate"><Languages :size="15" />{{ completed && completed < plan.units.length ? t("继续翻译未完成部分") : t("翻译") }}</Button><Button v-if="!config && !loading" variant="outline" @click="loadConfig">{{ t("重新读取配置") }}</Button></div></footer>
  </section>
</template>
