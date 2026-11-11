<script setup lang="ts">
import InfoTip from '@/shared/ui/InfoTip.vue'
import { t, te } from '@/shared/i18n/i18n'
import { handleEscape } from '@/shared/ui/escape'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import { onlineTranslationReady, targetLanguages, translationPresets, type AiSettings } from '@/features/recognition/ai'
import { ArrowLeft, ArrowLeftRight, Check, ClipboardPaste, Copy, Languages, LoaderCircle, RefreshCw, RotateCcw, Trash2, WrapText, X } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { updateSettings } from '@/features/settings/settings-write'
import { automaticTarget, detectTextLanguage } from '@/features/recognition/translation-direction'

const emit = defineEmits<{ close: []; back: []; busy: [value: boolean] }>()
// pasteable=false：贴图窗口无权读取剪贴板原文，嵌入识别工作区时隐藏该按钮。
const props = withDefaults(defineProps<{ workspace?: boolean; standalone?: boolean; autoTranslate?: boolean; pasteable?: boolean }>(), { workspace: false, autoTranslate: true, pasteable: true })
const source = defineModel<string>('text', { required: true })
const translated = ref('')
const busy = ref(false), error = ref(''), message = ref(''), target = ref('')
watch(busy, value => emit('busy', value))
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
const providerLabel = computed(() => !config.value ? '翻译服务' : local.value ? '本地翻译' : config.value.provider === 'deepl' ? 'DeepL' : translationPresets.find(item => item.endpoint === config.value?.endpoint)?.label ?? '兼容服务')
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
    if (config.value.provider !== 'local-ctranslate2' && !onlineTranslationReady(config.value)) error.value = '请先在“翻译”中选择服务，或在“服务管理”中添加服务。'
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
  if (busy.value || !swapSource.value) return
  const previousSource = swapSource.value
  if (!previousSource) return
  autoDirection.value = false
  if (translated.value) source.value = translated.value
  sourceLanguage.value = target.value; target.value = previousSource; invalidate()
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
/** 原文和译文复用复制通道，按钮分别标识内容，避免误复制。 */
async function copyText(value: string, success: string) {
  if (copying.value || !value.trim()) return
  copying.value = true; message.value = ''; error.value = ''
  try { await api.copyOcrText(value); message.value = success }
  catch (e) { error.value = String(e) }
  finally { copying.value = false }
}
function copy() { return copyText(translated.value, '译文已复制。') }
function mergeLines() {
  source.value = source.value.replace(/-\s*\r?\n\s*/g, '').replace(/\r?\n\s*/g, ' ')
  invalidate()
}
/** 结果卡片的回译操作以当前译文为原文，交换方向后立即提交。 */
async function reverse() {
  if (busy.value || !translated.value || !swapSource.value) return
  swap()
  // defineModel 的新原文需要父窗口回传；等待更新后再提交，避免回译仍发送旧原文。
  await nextTick()
  if (!disposed) await translate()
}
function key(event: KeyboardEvent) {
  if (props.standalone && event.key === 'Enter' && !event.shiftKey && !event.isComposing && !event.repeat
    && (event.target as HTMLElement)?.tagName === 'TEXTAREA' && (event.target as HTMLElement).id === 'translation-source') {
    event.preventDefault(); void translate(); return
  }
  if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !event.isComposing) { event.preventDefault(); void translate(); return }
  if (props.workspace) { handleEscape(event, () => emit('close')); return }
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
  <section class="translation-panel" :aria-label="t('文字翻译')" @keydown.stop="key">
    <header v-if="!workspace" class="panel-heading"><span>{{ t('文字翻译') }}</span><Button variant="ghost" :aria-label="t('关闭翻译')" @click="emit('close')"><X :size="16" /></Button></header>
    <div class="source-card">
      <label class="sr-only" for="translation-source">{{ t('原文') }}</label>
      <textarea id="translation-source" ref="input" v-model="source" :disabled="busy || readingClipboard" :aria-label="t('原文，可编辑')" :placeholder="t('输入需要翻译的文字…')" spellcheck="false" @input="invalidate" />
      <div class="source-tools">
        <div class="icon-group">
          <Button variant="ghost" :disabled="!source.trim() || copying" :title="t('复制原文')" :aria-label="t('复制原文')" @click="copyText(source, '原文已复制。')"><Copy :size="15" /></Button>
          <Button v-if="pasteable" variant="ghost" :disabled="busy || readingClipboard" :title="t('读取剪贴板')" :aria-label="t('读取剪贴板')" @click="paste"><ClipboardPaste :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || !source.trim()" :title="t('合并换行')" :aria-label="t('合并换行')" @click="mergeLines"><WrapText :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || readingClipboard || !source" :title="t('清空')" :aria-label="t('清空')" @click="source = ''; invalidate()"><Trash2 :size="15" /></Button>
          <slot name="source-actions" />
        </div>
        <span v-if="['zh-CN', 'en'].includes(detectedLanguage)" class="detected-language">{{ t(detectedLanguage === 'en' ? '英语' : '简体中文') }}</span>
        <Button class="translate-action" variant="ghost" :disabled="busy || copying || readingClipboard || savingDefault || !canTranslate || !source.trim()" :aria-label="translated ? t('重新翻译') : t('翻译')" :title="standalone ? t('Enter 翻译，Shift+Enter 换行') : t('Ctrl+Enter 翻译')" @click="translate"><Languages :size="18" /></Button>
      </div>
    </div>
    <div class="language-area">
      <div class="language-selectors">
        <SelectControl v-model="sourceLanguage" :options="sourceOptions" :disabled="busy || loading || !config" :aria-label="t('原文语言')" @update:model-value="sourceChanged" />
        <Button variant="ghost" :disabled="busy || !swapSource" :title="t('交换语言和文字')" :aria-label="t('交换语言和文字')" @click="swap"><ArrowLeftRight :size="17" /></Button>
        <SelectControl v-model="target" :options="targetOptions" :disabled="busy || loading || savingDefault || !config" :aria-label="t('目标语言')" @update:model-value="targetChanged" />
      </div>
      <div class="language-options">
        <label class="translation-auto" :title="autoDirection && detectedLanguage === 'mixed' ? t('中英比例接近，使用默认目标语言。') : undefined"><input v-model="autoDirection" type="checkbox" :disabled="busy || loading || !config" @change="automaticChanged" />{{ t('中英自动互译') }}</label>
        <Button v-if="workspace && !standalone" variant="ghost" :disabled="busy || loading || !config || savingDefault || target === config.targetLanguage" @click="saveDefault">{{ t('设为默认语言') }}</Button>
      </div>
    </div>
    <section class="provider-card" :aria-label="t('译文')" :aria-busy="busy">
      <header class="provider-heading">
        <span class="provider-name"><Languages :size="16" />{{ t(providerLabel) }}</span><InfoTip v-if="config && !local" :text="t('在线翻译会发送原文，可能产生费用。')" />
        <span v-if="busy" class="translation-status" role="status"><LoaderCircle :size="14" class="translation-spinner" />{{ t('翻译中') }}</span>
        <span v-else-if="loading" class="translation-status" role="status">{{ t('正在读取配置…') }}</span>
        <span v-else-if="translated" class="translation-success" role="status"><Check :size="13" />{{ t('已完成') }}</span>
        <Button variant="ghost" :disabled="busy || loading" :title="t('重新读取配置')" :aria-label="t('重新读取配置')" @click="loadConfig"><RefreshCw :size="14" /></Button>
      </header>
      <label class="sr-only" for="translation-output">{{ t('译文') }} · {{ t(targetLabel) }}</label>
      <textarea id="translation-output" :value="translated" readonly :aria-label="t('译文')" :placeholder="busy ? t('正在翻译，请稍候…') : t('译文将在这里显示')" />
      <p v-if="error" class="translation-error" role="alert">{{ te(error) }}</p>
      <footer class="provider-tools">
        <div class="icon-group">
          <Button variant="ghost" :disabled="!translated || copying" :title="t('复制译文')" :aria-label="t('复制译文')" @click="copy"><Copy :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || !translated || !swapSource" :title="t('回译')" :aria-label="t('回译')" @click="reverse"><ArrowLeftRight :size="15" /></Button>
          <Button variant="ghost" :disabled="busy || loading || !canTranslate || !source.trim()" :title="t('重新翻译')" :aria-label="t('重新翻译')" @click="translate"><RotateCcw :size="15" /></Button>
        </div>
        <Button v-if="busy" variant="ghost" @click="cancel">{{ t('取消翻译') }}</Button>
        <span v-else-if="message" class="translation-success" role="status"><Check :size="13" />{{ t(message) }}</span>
        <span v-else class="target-language">{{ t(targetLabel) }}</span>
      </footer>
    </section>
    <Button v-if="!workspace" class="back-action" variant="ghost" @click="emit('back')"><ArrowLeft :size="15" />{{ t('返回识别') }}</Button>
  </section>
</template>

<style scoped>
/* 参考 Pot 的纵向浮窗：原文、语言条、服务结果卡；所有操作贴近对应文字。 */
.translation-panel { --panel-bg:#f5f5f7; --card-bg:#fff; --bar-bg:#eeeef1; --panel-fg:#27272a; --panel-muted:#71717a; --panel-border:#e4e4e7; display:flex; flex-direction:column; gap:8px; min-height:0; color:var(--panel-fg); font-size:13px; }
:global([data-theme=dark]) .translation-panel { --panel-bg:#18181b; --card-bg:#27272a; --bar-bg:#323237; --panel-fg:#f4f4f5; --panel-muted:#a1a1aa; --panel-border:#3f3f46; }
.source-card, .provider-card { flex-shrink:0; border-radius:10px; overflow:hidden; background:var(--card-bg); }
.translation-panel textarea { display:block; width:100%; border:0; border-radius:0; padding:12px; resize:vertical; font:inherit; font-size:14px; line-height:1.65; background:transparent; color:inherit; user-select:text; }
.translation-panel textarea::placeholder { color:var(--panel-muted); }
.translation-panel textarea { scrollbar-width:none; }
.translation-panel textarea::-webkit-scrollbar { display:none; }
.source-card textarea { min-height:104px; height:140px; max-height:40vh; }
.provider-card textarea { min-height:120px; height:180px; max-height:50vh; }
.source-tools, .provider-tools, .provider-heading, .panel-heading { display:flex; align-items:center; gap:6px; padding:4px 8px; }
.source-tools, .provider-tools { flex-wrap:wrap; }
.icon-group { display:flex; align-items:center; gap:2px; }
.translation-panel :deep(button), .translation-panel :deep(.source-action) { min-width:28px; min-height:28px; height:28px; padding:5px; border-radius:6px; color:var(--panel-muted); background:transparent; }
.translation-panel :deep(button:hover:enabled) { background:var(--bar-bg); color:var(--panel-fg); }
.translation-panel :deep(button:disabled) { opacity:.4; }
.translation-panel textarea:focus-visible, .translation-panel :deep(button:focus-visible) { outline:2px solid #2563eb; outline-offset:-2px; }
.source-tools .translate-action { margin-left:auto; color:#2563eb; }
.detected-language { font-size:11px; color:var(--panel-muted); border:1px solid var(--panel-border); padding:1px 6px; border-radius:10px; }
.language-area { flex-shrink:0; }
.language-selectors { display:grid; grid-template-columns:minmax(0,1fr) 32px minmax(0,1fr); align-items:center; gap:4px; padding:2px 4px; background:var(--bar-bg); border-radius:10px; }
.language-selectors :deep(.select-trigger) { width:100%; min-width:0; height:32px; border:0; background:transparent; color:var(--panel-fg); }
.language-selectors :deep(.select-trigger:last-child) { text-align:right; }
.language-options { display:flex; align-items:center; justify-content:space-between; gap:8px; padding:6px 4px 0; }
.translation-auto { display:inline-flex; align-items:center; gap:6px; font-size:11px; color:var(--panel-muted); }
.translation-auto input { width:13px; height:13px; }
.provider-heading { background:var(--bar-bg); min-height:32px; }
.provider-name { display:inline-flex; align-items:center; gap:6px; font-weight:600; margin-right:auto; }
.provider-name svg { color:#2563eb; }
.translation-status, .translation-success { display:inline-flex; align-items:center; gap:4px; font-size:11px; }
.translation-status, .target-language { color:var(--panel-muted); }
.translation-success { color:var(--ui-success-fg, #24834b); }
.provider-tools > :last-child:not(.icon-group) { margin-left:auto; }
.target-language { font-size:11px; }
.translation-error { margin:0; padding:8px 12px; font-size:12px; color:#b42318; overflow-wrap:anywhere; }
:global([data-theme=dark]) .translation-error { color:#fda29b; }
.translation-hint { margin:4px 4px 0; color:var(--panel-muted); font-size:11px; line-height:1.5; }
.online-hint { padding-bottom:8px; }
.panel-heading { justify-content:space-between; }
.back-action { align-self:flex-start; }
.translation-spinner { animation:translation-spin 1s linear infinite; }
@keyframes translation-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion:reduce) { .translation-spinner { animation:none; } }
</style>
