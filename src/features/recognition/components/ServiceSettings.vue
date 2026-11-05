<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check, Plus } from 'lucide-vue-next'
import { api } from '@/services/api'
import { updateSettings } from '@/features/settings/settings-write'
import { defaultSettings } from '@/features/settings/settings'
import { cloudFirst, connectionMode, serviceKind, servicePresetId, servicePresets, paddleJobsEndpoint, paddleModels, isPaddleJobsEndpoint, supportsOcr, supportsTranslation, validateService, type RecognitionService } from '../services'
import { t, te } from '@/shared/i18n/i18n'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import InfoTip from '@/shared/ui/InfoTip.vue'
import ProviderMenu, { type ProviderMenuItem } from './ProviderMenu.vue'

/** OCR 页复用同一配置表单，避免再要求用户跳转到服务管理。 */
const props = defineProps<{ embedded?: boolean; presetId?: string; serviceId?: string }>()
const emit = defineEmits<{ saved: [service: RecognitionService]; busyChange: [busy: boolean] }>()
const settings = ref(defaultSettings()), ready = ref(false), busy = ref(false), error = ref(''), message = ref('')
const draft = ref<RecognitionService | null>(null), preset = ref('groq'), secret = ref('')
const customPaddleEndpoint = ref('')
const paddleEndpointChoice = computed(() => draft.value?.endpoint === paddleJobsEndpoint ? 'official' : 'custom')
const paddleModelOptions = computed(() => (isPaddleJobsEndpoint(draft.value?.endpoint ?? '') ? [...paddleModels] : ['PP-OCRv6', 'PP-OCRv5']).map(value => ({ value, label: value })))
function selectPaddleEndpoint(value: string) {
  if (!draft.value) return
  if (draft.value.endpoint !== paddleJobsEndpoint) customPaddleEndpoint.value = draft.value.endpoint
  draft.value.endpoint = value === 'official' ? paddleJobsEndpoint : customPaddleEndpoint.value
  endpointChanged()
}
const selectedMenu = ref('preset:groq')
// 仅在当前页面内保留未保存的非敏感表单；切换连接立即清空密钥输入。
const drafts = new Map<string, RecognitionService>()
const presetOptions = computed(() => servicePresets.map(item => ({ value: `preset:${item.id}`, label: item.id === 'custom' ? t('自定义') : item.id === 'paddle-ocr' ? 'Paddle API' : item.id === 'deepl' ? 'DeepL' : item.name, connection: item.id === 'custom' ? undefined : 'online' as const })))
const savedOptions = computed(() => settings.value.recognitionServices.map(item => ({ value: `service:${item.id}`, label: item.name, description: item.model || item.protocol, connection: connectionMode(item.endpoint), active: active(item.id), presetId: servicePresetId(item) })))
// 每个预设只在没有对应连接时出现；同平台的多个真实账号分别保留。
const menuOptions = computed(() => {
  const options: ProviderMenuItem[] = []
  for (const item of presetOptions.value) {
    const saved = savedOptions.value.filter(service => `preset:${service.presetId}` === item.value)
    options.push(...(saved.length ? saved : [item]))
  }
  return cloudFirst(options)
})
const visibleServices = computed(() => props.embedded || draft.value ? [] : settings.value.recognitionServices.filter(item => selectedMenu.value === `service:${item.id}`))
const statuses = ref<Record<string, boolean | null>>({})
let disposed = false, revision = 0
watch(busy, value => emit('busyChange', value), { flush: 'sync' })
async function load() {
  error.value = ''
  try {
    const value = await api.status()
    if (disposed) return
    settings.value = value.settings; ready.value = true
    const existing = props.serviceId ? settings.value.recognitionServices.find(item => item.id === props.serviceId) : undefined
    if (existing) {
      preset.value = servicePresetId(existing)
      edit(existing)
    } else if (props.presetId && servicePresets.some(item => item.id === props.presetId)) {
      preset.value = props.presetId; add()
    } else if (settings.value.recognitionServices[0]) {
      const first = settings.value.recognitionServices[0]
      selectedMenu.value = `service:${first.id}`; preset.value = servicePresetId(first)
    }
    const version = revision
    await Promise.all(settings.value.recognitionServices.map(async service => {
      try {
        const configured = await api.recognitionServiceStatus(service.id)
        if (!disposed && version === revision) statuses.value[service.id] = configured
      } catch { if (!disposed && version === revision) statuses.value[service.id] = null }
    }))
  } catch (e) { if (!disposed) error.value = String(e) }
}
onMounted(load)
onUnmounted(() => { disposed = true; revision++; secret.value = ''; drafts.clear() })
function add() {
  customPaddleEndpoint.value = ''
  if (busy.value || !ready.value) return
  selectedMenu.value = `preset:${preset.value}`
  const selected = servicePresets.find(item => item.id === preset.value)!
  draft.value = { id: crypto.randomUUID(), name: selected.name, protocol: selected.protocol, endpoint: selected.endpoint, model: selected.model, vision: selected.vision }
  secret.value = ''; message.value = ''; error.value = ''; revision++
}
function edit(service: RecognitionService) { customPaddleEndpoint.value = service.endpoint === paddleJobsEndpoint ? '' : service.endpoint; preset.value = servicePresetId(service); selectedMenu.value = `service:${service.id}`; draft.value = { ...service }; secret.value = ''; message.value = ''; error.value = ''; revision++ }
function cancel() { drafts.delete(selectedMenu.value); draft.value = null; secret.value = ''; revision++ }
function selectMenu(value: string) {
  if (busy.value || !ready.value || (value === selectedMenu.value && draft.value)) return
  if (draft.value) drafts.set(selectedMenu.value, { ...draft.value })
  selectedMenu.value = value
  if (value.startsWith('preset:')) {
    preset.value = value.slice(7); add()
  } else {
    const service = settings.value.recognitionServices.find(item => item.id === value.slice(8))
    if (service) edit(service)
  }
  const cached = drafts.get(value)
  if (cached) draft.value = { ...cached }
}
function endpointChanged() {
  secret.value = ''; revision++
  if (draft.value?.protocol === 'paddle-ocr' && !isPaddleJobsEndpoint(draft.value.endpoint) && !['PP-OCRv5', 'PP-OCRv6'].includes(draft.value.model)) draft.value.model = 'PP-OCRv6'
}
function active(id: string) { return (settings.value.ocr.provider === 'service' && settings.value.ocr.serviceId === id) || (settings.value.ai.provider !== 'local-ctranslate2' && settings.value.ai.serviceId === id) }
async function save() {
  if (!draft.value || busy.value) return
  const service = { ...draft.value }, key = secret.value, version = revision
  const invalid = validateService(service)
  if (invalid) { error.value = invalid; return }
  const previous = settings.value.recognitionServices.find(item => item.id === service.id)
  if (props.embedded && !key && (statuses.value[service.id] !== true || previous?.endpoint !== service.endpoint)) { error.value = '请先填写 API Key / Token，再启用在线 OCR。'; return }
  if (key && (key.length > 2048 || /\p{Cc}/u.test(key))) { error.value = '密钥过长或含控制字符。'; return }
  busy.value = true; error.value = ''; message.value = ''
  try {
    const saved = await updateSettings(current => ({ ...current, recognitionServices: current.recognitionServices.some(item => item.id === service.id) ? current.recognitionServices.map(item => item.id === service.id ? service : item) : [...current.recognitionServices, service] }))
    if (!disposed) settings.value = saved
    if (key) await api.recognitionServiceCredential(service.id, service.endpoint, key)
    const configured = await api.recognitionServiceStatus(service.id)
    if (!disposed && version === revision) { statuses.value[service.id] = configured; cancel(); selectedMenu.value = `service:${service.id}`; message.value = '服务已保存。'; emit('saved', service) }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function remove(service: RecognitionService) {
  if (busy.value || active(service.id)) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    // 先删除凭据；失败时保留连接，以便用户重试而不留下不可管理的密钥。
    await api.recognitionServiceCredential(service.id, service.endpoint, null)
    const saved = await updateSettings(current => ({ ...current, recognitionServices: current.recognitionServices.filter(item => item.id !== service.id) }))
    if (!disposed) { settings.value = saved; if (draft.value?.id === service.id) cancel(); drafts.delete(`service:${service.id}`); selectedMenu.value = 'preset:groq'; preset.value = 'groq'; message.value = '服务已删除。' }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function deleteKey() {
  if (!draft.value || busy.value) return
  const service = settings.value.recognitionServices.find(item => item.id === draft.value!.id)
  if (!service) return
  busy.value = true; error.value = ''; message.value = ''
  try { await api.recognitionServiceCredential(service.id, service.endpoint, null); if (!disposed) { statuses.value[service.id] = false; secret.value = ''; message.value = '密钥已删除。' } }
  catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function portal() {
  try { await api.openRecognitionPortal(preset.value) } catch (e) { error.value = String(e) }
}
</script>

<template>
  <section class="service-settings" :class="{ 'recognition-settings-layout': !embedded }">
    <aside v-if="!embedded" class="recognition-settings-sidebar">
      <ProviderMenu :model-value="selectedMenu" :options="menuOptions" :label="t('服务')" :disabled="busy || !ready" @update:model-value="selectMenu" />
    </aside>
    <div class="recognition-settings-content">
    <div class="toolbar"><Button v-if="!embedded && !draft" :disabled="busy || !ready || !!draft || settings.recognitionServices.length >= 64" variant="outline" @click="add"><Plus :size="14" />{{ t('添加服务') }}</Button><Button v-if="(embedded || selectedMenu.startsWith('preset:')) && preset !== 'custom'" variant="ghost" :title="t('当前通过 API Key / Token 接入；浏览器登录用于获取凭据，不会自动授权 Snap Desk。')" @click="portal">{{ t('获取 API Key / Token') }}</Button></div>
    <p v-if="!ready" role="status">{{ t('正在读取配置…') }}</p>
    <p v-else-if="!embedded && !draft && !settings.recognitionServices.length" class="hint">{{ t('尚未添加服务。') }}</p>
    <div v-for="service in visibleServices" :key="service.id" class="service-row">
      <div><strong>{{ service.name }}</strong><small :title="service.endpoint">{{ service.model || t(serviceKind(service)) }} · {{ [supportsOcr(service) && t('文字识别'), supportsTranslation(service) && t('翻译')].filter(Boolean).join(' / ') }}</small></div>
      <span role="status" :class="{ 'status-ok': statuses[service.id] === true }"><Check v-if="statuses[service.id] === true" :size="14" />{{ statuses[service.id] === true ? t('已配置密钥') : statuses[service.id] === false ? t('未配置密钥') : t('凭据状态未读取') }}</span>
      <div class="actions"><Button variant="outline" :disabled="busy" @click="edit(service)">{{ t('编辑') }}</Button><Button variant="ghost" :disabled="busy || active(service.id)" @click="remove(service)">{{ t('删除') }}</Button></div>
    </div>
    <form v-if="draft" class="editor" @submit.prevent="save">
      <fieldset :disabled="busy">
        <label>{{ t('服务名称') }}<input v-model.trim="draft.name" maxlength="80" /></label>
        <template v-if="draft.protocol === 'paddle-ocr'">
          <label>{{ t('完整接口地址') }}<SelectControl :model-value="paddleEndpointChoice" :aria-label="t('Paddle 接口地址')" :title="draft.endpoint" :options="[{ value: 'official', label: t('官方异步接口（推荐）') }, { value: 'custom', label: t('同步接口（自定义地址）') }]" @update:model-value="selectPaddleEndpoint" /></label>
          <label v-if="paddleEndpointChoice === 'custom'">{{ t('同步接口地址') }}<input v-model.trim="draft.endpoint" type="url" autocomplete="off" :placeholder="t('填写控制台提供的完整 /ocr 地址')" @input="endpointChanged" /></label>
          <label>{{ t('模型名称') }}<SelectControl v-model="draft.model" :aria-label="t('Paddle 模型')" :options="paddleModelOptions" /></label>
        </template>
        <label v-else>{{ t('完整接口地址') }}<input v-model.trim="draft.endpoint" type="url" autocomplete="off" @input="endpointChanged" /></label>
        <label v-if="draft.protocol !== 'deepl' && draft.protocol !== 'paddle-ocr'">{{ draft.protocol === 'ocr-space' ? t('OCR.space 引擎（1 / 2 / 3）') : t('模型名称') }}<input v-model.trim="draft.model" maxlength="200" /></label>
        <label v-if="draft.protocol === 'openai-compatible'" class="checkbox"><input v-model="draft.vision" type="checkbox" />{{ t('此模型支持图片输入，可用于 OCR') }}</label>
        <label>API Key / Token<input v-model="secret" type="password" autocomplete="new-password" :placeholder="t('留空保留现有密钥')" /></label>
        <div class="actions"><Button type="submit">{{ busy ? t('正在保存…') : embedded ? t('保存并用于 OCR') : t('保存服务') }}</Button><Button v-if="!embedded" type="button" variant="outline" @click="cancel">{{ t('取消') }}</Button><InfoTip v-if="connectionMode(draft.endpoint) === 'online'" :text="supportsOcr(draft) ? t('OCR 会将选区图片发送到此服务，可能产生费用。') : t('在线翻译会发送原文，可能产生费用。')" /><Button v-if="settings.recognitionServices.some(item => item.id === draft?.id)" type="button" variant="ghost" @click="deleteKey">{{ t('删除已保存密钥') }}</Button></div>
      </fieldset>
    </form>
    <p v-if="message" class="status-ok" role="status"><Check :size="14" />{{ t(message) }}</p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}<Button v-if="!ready" variant="ghost" @click="load">{{ t('重试') }}</Button></p>
    </div>
  </section>
</template>

<style scoped>
.service-settings { font-size:13px; }
.toolbar,.actions { display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
.toolbar { margin-bottom:8px; }
.service-row { display:flex; flex-wrap:wrap; gap:12px; padding:8px 0; border-bottom:1px solid var(--desk-border,#e1e4e8); align-items:center; }
small { display:block; margin-top:4px; color:var(--desk-muted,#57606a); overflow-wrap:anywhere; }
.hint { color:var(--desk-muted,#57606a); font-size:12px; line-height:1.6; }
.editor { padding-top:0; max-width:720px; }
fieldset { padding:0; border:0; min-width:0; }
label { display:grid; grid-template-columns:124px minmax(0,1fr); gap:12px; align-items:center; margin:8px 0; }
input:not([type=checkbox]) { width:100%; height:32px; padding:4px 8px; min-width:0; }
.checkbox { display:flex; }
input:focus-visible { outline:2px solid #2563eb; outline-offset:2px; }
.error { color:#b91c1c; overflow-wrap:anywhere; }
@media (max-width:650px) { .service-row { grid-template-columns:minmax(0,1fr); gap:8px; } label { grid-template-columns:1fr; gap:6px; } }
</style>
