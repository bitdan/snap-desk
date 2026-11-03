<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import { onlineTranslationReady, targetLanguages, type AiSettings } from '@/features/recognition/ai'
import { ArrowLeft, Check, Copy, Languages, LoaderCircle, X } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { updateSettings } from '@/features/settings/settings-write'
import { automaticTarget, detectTextLanguage } from '@/features/recognition/translation-direction'
import '@/features/recognition/components/recognition-results.css'

const emit = defineEmits<{ close: []; back: [] }>()
const props = withDefaults(defineProps<{ workspace?: boolean; autoTranslate?: boolean }>(), { workspace: false, autoTranslate: true })
const source = defineModel<string>('text', { required: true })
const translated = ref('')
const busy = ref(false), error = ref(''), message = ref(''), target = ref('')
const loading = ref(false), copying = ref(false)
const sourceLanguage = ref(''), savingDefault = ref(false), readingClipboard = ref(false)
const input = ref<HTMLTextAreaElement | null>(null)
const config = ref<AiSettings | null>(null)
const autoDirection = ref(true)
const detectedLanguage = computed(() => detectTextLanguage(source.value))
const local = computed(() => config.value?.provider === 'local-ctranslate2')
const languages = computed(() => local.value ? targetLanguages.slice(0, 2) : targetLanguages)
const sourceOptions = computed(() => [{ value: '', label: t('检测语言') }, ...languages.value.map(item => ({ ...item, label: t(item.label) }))])
const targetOptions = computed(() => languages.value.map(item => ({ ...item, label: t(item.label) })))
const canTranslate = computed(() => !!config.value && (!loading.value && (local.value || onlineTranslationReady(config.value))))
const targetLabel = computed(() => targetLanguages.find(item => item.value === (target.value || config.value?.targetLanguage))?.label ?? '目标语言')
const swapSource = computed(() => sourceLanguage.value || (autoDirection.value && ['zh-CN', 'en'].includes(detectedLanguage.value)
  ? detectedLanguage.value : local.value ? (target.value === 'en' ? 'zh-CN' : 'en') : ''))
watch(source, () => applyAutomaticDirection(), { flush: 'sync' })
function applyAutomaticDirection() {
  if (autoDirection.value && !sourceLanguage.value && config.value && !busy.value) {
    target.value = automaticTarget(detectedLanguage.value, config.value.targetLanguage)
  }
}
function automaticChanged() {
  if (autoDirection.value) { sourceLanguage.value = ''; applyAutomaticDirection() }
  invalidate()
}
function sourceChanged() {
  autoDirection.value = !sourceLanguage.value && ['zh-CN', 'en'].includes(target.value)
  applyAutomaticDirection(); invalidate()
}
function targetChanged() { autoDirection.value = false; invalidate() }
let requestId: string | null = null, disposed = false, cancelled = false
/** 翻译入口代表执行请求；读取配置后使用指定引擎自动翻译，不切换服务。 */
async function loadConfig() {
  if (loading.value) return
  loading.value = true; invalidate()
  let loaded = false
  try {
    const value = await api.aiTranslationConfig()
    if (disposed) return
    config.value = value
    target.value = value.targetLanguage
    if (!languages.value.some(item => item.value === sourceLanguage.value)) sourceLanguage.value = ''
    autoDirection.value = !sourceLanguage.value && ['zh-CN', 'en'].includes(value.targetLanguage)
    applyAutomaticDirection()
    loaded = true
    if (config.value.provider !== 'local-ctranslate2' && !onlineTranslationReady(config.value)) error.value = '请先在“识别与翻译”中配置在线服务。'
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { loading.value = false }
  if (!disposed && loaded && props.autoTranslate) await translate()
}
onMounted(() => { void nextTick(() => input.value?.focus()); void loadConfig() })
onUnmounted(() => { disposed = true; if (requestId) void api.cancelAiJob(requestId).catch(() => {}) })
async function translate() {
  if (busy.value || readingClipboard.value || savingDefault.value || !canTranslate.value || !source.value.trim() || !config.value) return
  applyAutomaticDirection()
  if (sourceLanguage.value === target.value) { error.value = '原文语言和目标语言不能相同。'; return }
  if (new TextEncoder().encode(source.value).length > 32_000) { error.value = '翻译文字超过 32 KB，请减少文字后重试。'; return }
  const id = crypto.randomUUID(); requestId = id; cancelled = false
  busy.value = true; error.value = ''; message.value = ''; translated.value = ''
  try {
    // 默认方向保留单文本调用；临时方向和指定原文语言通过现有批量契约传递。
    const result = !sourceLanguage.value && target.value === config.value.targetLanguage
      ? await api.translateText(id, source.value, true, config.value)
      : await api.translateTexts(id, { texts: [source.value], sourceLanguage: sourceLanguage.value || null, targetLanguage: target.value }, true, config.value)
    if (!disposed && !cancelled && requestId === id && result.requestId === id) {
      translated.value = 'translatedText' in result ? result.translatedText : result.translations[0]?.translatedText ?? ''
    }
  } catch (e) { if (!disposed && !cancelled && requestId === id) error.value = String(e) }
  finally { if (requestId === id) { busy.value = false; requestId = null } }
}
/** 修改原文或方向即清除旧结果，避免复制到与当前语言不匹配的译文。 */
function invalidate() { translated.value = ''; message.value = ''; error.value = '' }
function swap() {
  if (busy.value || !translated.value) return
  const previousSource = swapSource.value
  if (!previousSource) return
  autoDirection.value = false
  source.value = translated.value; sourceLanguage.value = target.value; target.value = previousSource; invalidate()
}
async function paste() {
  if (busy.value || readingClipboard.value) return
  readingClipboard.value = true
  try { const text = await api.readTranslationInput('clipboard'); if (!disposed && text !== null) { source.value = text; invalidate() } }
  catch (e) { if (!disposed) error.value = String(e) }
  finally { readingClipboard.value = false }
}
async function saveDefault() {
  if (savingDefault.value || !config.value) return
  savingDefault.value = true; error.value = ''; message.value = ''
  const language = target.value
  try {
    const saved = await updateSettings(current => ({ ...current, ai: { ...current.ai, targetLanguage: language } }))
    if (!disposed) { config.value = saved.ai; message.value = '默认目标语言已保存。' }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { savingDefault.value = false }
}
async function cancel() {
  if (!requestId) return
  cancelled = true
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
  if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !event.isComposing) { event.preventDefault(); void translate(); return }
  if (props.workspace) return
  if (handleEscape(event, () => emit('close'))) return
  if (event.key === 'Tab') {
    const nodes = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('textarea:not(:disabled), input:not(:disabled), button:not(:disabled)')]
    const first = nodes[0], last = nodes[nodes.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
  }
}
</script>

<template>
  <section class="recognition-result translation-result" :class="{ 'translation-workspace': workspace }" @keydown.stop="key">
    <header v-if="!workspace" class="result-header"><h2><Languages :size="17" />{{ t("文字翻译") }}</h2><Button class="result-close" variant="ghost" :title='t("关闭（Esc）")' :aria-label='t("关闭翻译")' @click="emit('close')"><X :size="16" /></Button></header>
    <div class="result-body">
      <div class="translation-languages">
        <SelectControl v-model="sourceLanguage" :options="sourceOptions" :disabled="busy || loading || !config" :aria-label="t('原文语言')" @update:model-value="sourceChanged" />
        <Button variant="ghost" :disabled="busy || !translated || !swapSource" :aria-label="t('交换语言和文字')" @click="swap">⇄</Button>
        <SelectControl v-model="target" :options="targetOptions" :disabled="busy || loading || savingDefault || !config" :aria-label="t('目标语言')" @update:model-value="targetChanged" />
        <label class="translation-auto"><input v-model="autoDirection" type="checkbox" :disabled="busy || loading || !config" @change="automaticChanged" />{{ t('中英自动互译') }}</label>
        <Button v-if="workspace" variant="ghost" :disabled="busy || loading || !config || savingDefault || target === config.targetLanguage" @click="saveDefault">{{ t('设为默认语言') }}</Button>
      </div>
      <p v-if="autoDirection && detectedLanguage === 'mixed'" class="translation-hint">{{ t('中英比例接近，使用默认目标语言。') }}</p>
      <div class="translation-stack">
        <div class="translation-text source-text"><div class="text-heading"><label for="translation-source">{{ t("原文") }}</label><span v-if="loading" role="status">{{ t("正在读取配置…") }}</span></div><textarea id="translation-source" ref="input" v-model="source" :disabled="busy || readingClipboard" :aria-label='t("原文，可编辑")' :placeholder='t("输入或修正需要翻译的文字")' spellcheck="false" @input="invalidate" /></div>
        <div class="translation-text translated-text" :aria-busy="busy"><div class="text-heading"><label for="translation-output">{{ t("译文") }} · {{ t(targetLabel) }}</label><span v-if="busy" class="result-progress" role="status"><LoaderCircle :size="13" class="result-spinner" />{{ t("翻译中") }}</span><span v-else-if="translated" class="result-success" role="status"><Check :size="13" />{{ t("已完成") }}</span></div><textarea id="translation-output" :value="translated" readonly :aria-label='t("译文")' :placeholder="busy ? t('正在翻译，请稍候…') : t('译文将在这里显示')" /></div>
      </div>
      <p v-if="workspace && config && !local" class="translation-hint">{{ t('在线翻译会发送原文，可能产生费用。') }}</p>
      <p v-if="error" class="result-error" role="alert">{{ te(error) }}</p>
    </div>
    <footer class="result-footer"><Button v-if="!workspace" variant="ghost" :disabled="copying" @click="emit('back')"><ArrowLeft :size="15" />{{ t("返回识别") }}</Button><template v-else><Button variant="outline" :disabled="busy || readingClipboard" @click="paste">{{ t('读取剪贴板') }}</Button><Button variant="ghost" :disabled="busy || readingClipboard" @click="source = ''; invalidate()">{{ t('清空') }}</Button></template><span v-if="message" class="result-success" role="status"><Check :size="14" />{{ t(message) }}</span><div class="result-actions"><Button v-if="busy" variant="outline" @click="cancel">{{ t("取消翻译") }}</Button><Button variant="outline" :disabled="!translated || copying" @click="copy"><Copy :size="15" />{{ t("复制译文") }}</Button><Button :disabled="busy || copying || readingClipboard || savingDefault || !canTranslate || !source.trim()" :title="t('Ctrl+Enter 翻译')" @click="translate"><LoaderCircle v-if="busy" :size="15" class="result-spinner" /><Languages v-else :size="15" />{{ busy ? t("翻译中…") : translated ? t("重新翻译") : t("翻译") }}</Button><Button v-if="!loading" variant="ghost" :disabled="busy" @click="loadConfig">{{ t("重新读取配置") }}</Button></div></footer>
  </section>
</template>

<style scoped>
.translation-languages { display:flex; align-items:center; flex-wrap:wrap; gap:8px; }
.translation-languages :deep(.select-trigger) { width:150px; }
.translation-auto { display:inline-flex; align-items:center; gap:6px; font-size:12px; }
.translation-workspace { flex:1 0 420px; height:auto; min-height:420px; aspect-ratio:auto; border:0; border-radius:0; background:var(--desk-bg, #fff); color:var(--desk-text, #24292f); }
.translation-workspace .result-body { padding:0 0 12px; }
.translation-workspace .result-footer { padding:8px 0; }
.translation-workspace .translation-stack { grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(140px,1fr) minmax(140px,1fr); min-height:288px; }
.translation-workspace .translated-text textarea { font-size:14px; font-weight:400; }
.translation-workspace .translation-text { background:var(--desk-soft, #f6f7f8); }
.translation-workspace textarea { color:inherit; }
.translation-workspace .result-footer { border-color:var(--desk-border, #e1e4e8); }
.translation-hint { margin:0; color:var(--ui-muted-fg, #57606a); font-size:12px; }
</style>
