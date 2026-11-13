<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check } from 'lucide-vue-next'
import { api } from '@/services/api'
import { updateSettings } from '@/features/settings/settings-write'
import { defaultSettings } from '@/features/settings/settings'
import { cloudFirst, connectionMode, serviceKind, servicePresetId, servicePresets, paddleJobsEndpoint, paddleModels, isPaddleJobsEndpoint, supportsOcr, supportsTranslation, validateService, type RecognitionService } from '../services'
import { t, te } from '@/shared/i18n/i18n'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import type { ProviderMenuItem } from './ProviderMenu.vue'
import { brandIcons } from '@/shared/ui/brand-icons'
import { providerIcon } from '../provider-icons'

/** OCR 页复用同一配置表单，避免再要求用户跳转到服务管理。 */
const props = defineProps<{ embedded?: boolean; presetId?: string; serviceId?: string }>()
const emit = defineEmits<{ saved: [service: RecognitionService]; busyChange: [busy: boolean]; openTab: [tab: 'ocr' | 'translation']; close: [] }>()
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
// 服务列表：每个供应商一行，已保存的账号替代对应预设；图标与识别、翻译页的引擎下拉共用。
const directory = computed(() => props.embedded ? [] : menuOptions.value.map(item => {
  const service = item.value.startsWith('service:') ? settings.value.recognitionServices.find(entry => `service:${entry.id}` === item.value) : undefined
  const presetId = service ? servicePresetId(service) : item.value.slice(7)
  const source = service ?? servicePresets.find(entry => entry.id === presetId)
  const uses = source ? [supportsOcr(source as RecognitionService) && t('文字识别'), supportsTranslation(source as RecognitionService) && t('翻译')].filter(Boolean).join(' · ') : ''
  const configured = service ? statuses.value[service.id] : undefined
  const status = !service ? t('未配置') : configured === true ? t('已配置密钥') : configured === false ? t('未配置密钥') : t('凭据状态未读取')
  const { icon, mono, tile } = providerIcon(item.value, item.label, presetId, service?.endpoint ?? source?.endpoint)
  return { ...item, service, mono, tile, icon, status: [status, service && active(service.id) ? t('当前使用') : '', uses].filter(Boolean).join(' · '), state: configured === true ? 'ready' : 'idle' }
}))
// 本地引擎只读状态并跳到对应标签页管理；读取失败显示“状态未读取”，不影响在线服务列表。
type LocalState = { resources?: Awaited<ReturnType<typeof api.resourceStatus>> | null; system?: Awaited<ReturnType<typeof api.systemOcrStatus>> | null; wechat?: Awaited<ReturnType<typeof api.wechatOcrStatus>> | null; translation?: Awaited<ReturnType<typeof api.translationModelStatus>> | null }
const localState = ref<LocalState>({})
async function loadLocal() {
  if (props.embedded) return
  const read = async <T,>(task: () => Promise<T>) => { try { return await task() } catch { return null } }
  const [resources, system, wechat, translation] = await Promise.all([read(() => api.resourceStatus()), read(() => api.systemOcrStatus()), read(() => api.wechatOcrStatus()), read(() => api.translationModelStatus())])
  if (!disposed) localState.value = { resources, system, wechat, translation }
}
const localEngines = computed(() => {
  const { resources, system, wechat, translation } = localState.value
  const unread = t('状态未读取'), ocr = t('文字识别'), translate = t('翻译')
  const ct2Ready = !!translation?.runtimeReady && !!translation.models?.some(pack => pack.ready)
  return [
    { key: 'ppocr', label: 'PaddleOCR', description: t('PP-OCRv6 · 离线'), mono: 'PP', tile: '#2563a8', icon: brandIcons.baidu, tab: 'ocr' as const, ready: resources?.ocrReady === true, unavailable: false,
      status: [resources == null ? unread : resources.ocrReady ? t('已就绪') : t('未下载识别资源'), ocr] },
    { key: 'windows', label: 'Windows OCR', description: t('系统自带 · 离线'), mono: 'W', tile: '#0067b8', icon: brandIcons.microsoft, tab: 'ocr' as const, ready: !!system?.supported && system.languages.length > 0, unavailable: system?.supported === false,
      status: [system == null ? unread : !system.supported ? t('仅 Windows 支持') : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装语言包'), ocr] },
    { key: 'wechat', label: 'WeChat OCR', description: t('实验性 · 离线'), mono: 'WX', tile: '#07a35a', icon: '', tab: 'ocr' as const, ready: wechat?.detected === true, unavailable: wechat?.supported === false,
      status: [wechat == null ? unread : wechat.supported === false ? t('当前平台不支持') : wechat.detected ? t('已找到 WeChat OCR 组件') : t('未找到 WeChat OCR 组件'), ocr] },
    { key: 'ct2', label: 'CTranslate2', description: t('本地中英互译 · 离线'), mono: 'CT', tile: '#475569', icon: '', tab: 'translation' as const, ready: ct2Ready, unavailable: false,
      status: [translation == null ? unread : ct2Ready ? t('已安装') : t('未安装'), translate] },
  ].map(item => ({ ...item, status: item.status.join(' · ') }))
})
onMounted(loadLocal)
const draftIcon = computed(() => providerIcon(draft.value ? `preset:${preset.value}` : '', draft.value?.name ?? '', preset.value, draft.value?.endpoint))
const dialogTitle = computed(() => !draft.value ? '' : settings.value.recognitionServices.some(item => item.id === draft.value!.id) ? t('编辑服务') : t('添加服务'))
function closeDialog() {
  if (busy.value) return
  cancel()
  if (props.embedded) emit('close')
}
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
  <section class="service-settings" :class="{ 'service-directory-page': !embedded }">
    <template v-if="!embedded">
      <h2 class="section-heading">{{ t('本地引擎') }}</h2>
      <div class="settings-card service-directory">
        <div v-for="item in localEngines" :key="item.key" class="service-item">
          <span class="service-mono" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" class="service-logo" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
          <span class="service-text">
            <span class="service-name">{{ item.label }}<small>{{ item.description }}</small></span>
            <span class="service-status" role="status"><i :class="item.ready ? 'ready' : 'idle'" aria-hidden="true" />{{ item.status }}</span>
          </span>
          <button type="button" class="service-action outlined" :disabled="item.unavailable" @click="emit('openTab', item.tab)">{{ t('管理') }}</button>
        </div>
      </div>
    </template>
    <h2 v-if="!embedded" class="section-heading">{{ t('在线服务') }}<button type="button" :disabled="busy || !ready || !!draft || settings.recognitionServices.length >= 64" @click="add">{{ t('添加服务') }}</button></h2>
    <p v-if="!ready" role="status" class="hint">{{ t('正在读取配置…') }}</p>
    <!-- 服务按供应商列出；新增与编辑在独立窗口中进行。 -->
    <div v-if="!embedded" class="settings-card service-directory">
      <div v-for="item in directory" :key="item.value" class="service-item" :class="{ selected: draft && selectedMenu === item.value }">
        <span class="service-mono" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" class="service-logo" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
        <span class="service-text">
          <span class="service-name">{{ item.label }}<small v-if="item.description">{{ item.description }}</small></span>
          <span class="service-status" role="status"><i :class="item.state" aria-hidden="true" />{{ item.status }}</span>
        </span>
        <template v-if="item.service">
          <button type="button" class="service-action" :data-provider="item.value" :aria-current="draft && selectedMenu === item.value ? 'page' : undefined" :disabled="busy || !ready" @click="selectMenu(item.value)">{{ t('编辑') }}</button>
          <button type="button" class="service-action" :disabled="busy || active(item.service.id)" @click="remove(item.service)">{{ t('删除') }}</button>
        </template>
        <button v-else type="button" class="service-action outlined" :data-provider="item.value" :aria-current="draft && selectedMenu === item.value ? 'page' : undefined" :disabled="busy || !ready" @click="selectMenu(item.value)">{{ t('配置') }}</button>
      </div>
    </div>
    <DetachedWindow :open="!!draft" :title="dialogTitle" :width="480" :height="420" :busy="busy" @close="closeDialog">
      <!-- 独立窗口不在主窗口 .app-shell 内，套用同一组设计变量，使控件与主界面一致。 -->
      <div v-if="draft" class="app-shell dialog-shell">
        <section class="service-dialog" role="dialog" :aria-label="dialogTitle">
          <header class="service-dialog-header">
            <span class="service-mono" :class="{ logo: draftIcon.icon }" :style="draftIcon.icon ? undefined : { background: draftIcon.tile }" aria-hidden="true"><span v-if="draftIcon.icon" class="service-logo" v-html="draftIcon.icon" /><template v-else>{{ draftIcon.mono }}</template></span>
            <h2>{{ dialogTitle }}</h2>
          </header>
          <form class="editor" @submit.prevent="save">
            <fieldset class="settings-card" :disabled="busy">
              <label class="setting-row dialog-field"><span>{{ t('服务名称') }}</span><input v-model.trim="draft.name" maxlength="80" /></label>
              <template v-if="draft.protocol === 'paddle-ocr'">
                <label class="setting-row dialog-field"><span>{{ t('完整接口地址') }}</span><SelectControl :model-value="paddleEndpointChoice" :aria-label="t('Paddle 接口地址')" :title="draft.endpoint" :options="[{ value: 'official', label: t('官方异步接口（推荐）') }, { value: 'custom', label: t('同步接口（自定义地址）') }]" @update:model-value="selectPaddleEndpoint" /></label>
                <label v-if="paddleEndpointChoice === 'custom'" class="setting-row dialog-field"><span>{{ t('同步接口地址') }}</span><input v-model.trim="draft.endpoint" type="url" autocomplete="off" :placeholder="t('填写控制台提供的完整 /ocr 地址')" @input="endpointChanged" /></label>
                <label class="setting-row dialog-field"><span>{{ t('模型名称') }}</span><SelectControl v-model="draft.model" :aria-label="t('Paddle 模型')" :options="paddleModelOptions" /></label>
              </template>
              <label v-else class="setting-row dialog-field"><span>{{ t('完整接口地址') }}</span><input v-model.trim="draft.endpoint" type="url" autocomplete="off" @input="endpointChanged" /></label>
              <label v-if="draft.protocol !== 'deepl' && draft.protocol !== 'paddle-ocr'" class="setting-row dialog-field"><span>{{ draft.protocol === 'ocr-space' ? t('OCR.space 引擎（1 / 2 / 3）') : t('模型名称') }}</span><input v-model.trim="draft.model" maxlength="200" /></label>
              <label class="setting-row dialog-field"><span>API Key / Token</span><input v-model="secret" type="password" autocomplete="new-password" :placeholder="t('留空保留现有密钥')" /></label>
              <label v-if="draft.protocol === 'openai-compatible'" class="setting-row dialog-switch"><span>{{ t('此模型支持图片输入，可用于 OCR') }}</span><input v-model="draft.vision" class="mini-switch" type="checkbox" role="switch" /></label>
            </fieldset>
            <p v-if="connectionMode(draft.endpoint) === 'online'" class="section-note">{{ supportsOcr(draft) ? t('OCR 会将选区图片发送到此服务，可能产生费用。') : t('在线翻译会发送原文，可能产生费用。') }}</p>
            <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
            <div class="actions">
              <button v-if="preset !== 'custom'" type="button" class="dialog-link" :title="t('当前通过 API Key / Token 接入；浏览器登录用于获取凭据，不会自动授权 Snap Desk。')" @click="portal">{{ t('获取 API Key / Token') }}</button>
              <button v-if="settings.recognitionServices.some(item => item.id === draft?.id)" type="button" class="dialog-link" :disabled="busy" @click="deleteKey">{{ t('删除已保存密钥') }}</button>
              <span class="actions-spacer" />
              <Button type="button" variant="outline" :disabled="busy" @click="closeDialog">{{ t('取消') }}</Button>
              <Button type="submit" :disabled="busy">{{ busy ? t('正在保存…') : embedded ? t('保存并用于 OCR') : t('保存服务') }}</Button>
            </div>
          </form>
        </section>
      </div>
    </DetachedWindow>
    <p v-if="!embedded" class="section-note">{{ t('密钥保存在系统凭据中，不随配置导出。') }}</p>
    <p v-if="message" class="status-ok" role="status"><Check :size="14" />{{ t(message) }}</p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}<Button v-if="!ready" variant="ghost" @click="load">{{ t('重试') }}</Button></p>
  </section>
</template>

<style scoped>
.service-settings { font-size:13px; }
.toolbar,.actions { display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
.toolbar { margin-bottom:8px; }
.hint { margin:0 4px 6px; color:var(--desk-muted,#5f6670); font-size:12px; line-height:1.6; }
.service-directory { display:flex; flex-direction:column; padding:0; }
.service-item { display:flex; align-items:center; gap:12px; min-height:54px; padding:8px 12px; border-top:1px solid var(--desk-sep,#eceef1); }
.service-directory > .service-item:first-child { border-top:0; }
.service-item.selected { background:color-mix(in srgb,var(--desk-soft,#eef1fc) 60%,transparent); }
.service-mono { width:28px; height:28px; flex:none; border-radius:7px; display:inline-flex; align-items:center; justify-content:center; color:#fff; font-size:11px; font-weight:700; letter-spacing:-.02em; }
.service-mono.logo { background:var(--desk-surface,#fff); box-shadow:inset 0 0 0 1px var(--desk-border,#e3e6ea); color:var(--desk-text,#1f2328); }
.service-logo { display:flex; font-size:18px; line-height:1; }
.service-logo :deep(svg) { width:18px; height:18px; }
.service-text { flex:1; min-width:0; display:flex; flex-direction:column; gap:2px; }
.service-name { display:flex; align-items:baseline; gap:8px; min-width:0; font-weight:600; }
.service-name small { font-size:11px; font-weight:400; color:var(--desk-muted,#5f6670); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.service-status { display:flex; align-items:center; gap:6px; min-width:0; font-size:12px; color:var(--desk-muted,#5f6670); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.service-status i { width:6px; height:6px; flex:none; border-radius:50%; background:var(--desk-line,#c3c8cf); }
.service-status i.ready { background:var(--ui-success-fg,#24834b); }
.service-action { height:24px; padding:0 10px; border:0; border-radius:6px; background:transparent; color:var(--desk-muted,#5f6670); font:inherit; font-size:12px; white-space:nowrap; cursor:pointer; }
.service-action:hover:enabled { background:var(--desk-hover,rgba(127,127,127,.12)); color:var(--desk-text,#1f2328); }
.service-action.outlined { box-shadow:inset 0 0 0 1px var(--desk-line,#d3d8de); background:var(--desk-surface,#fff); color:var(--desk-text,#1f2328); font-size:13px; }
.service-action:disabled { opacity:.45; cursor:default; }
.dialog-shell { height:auto; min-height:100%; display:block; overflow:auto; }
.service-dialog { padding:16px 20px 18px; }
.service-dialog-header { display:flex; align-items:center; gap:10px; margin-bottom:14px; }
.service-dialog-header h2 { margin:0; font-size:15px; font-weight:600; }
.dialog-field > span { flex:none; width:104px; color:var(--desk-text,#1f2328); }
.dialog-field > :is(input, .select-trigger) { flex:1; min-width:0; }
.dialog-field, .dialog-switch { flex-wrap:nowrap; margin:0; }
.service-dialog .actions { margin-top:16px; }
.actions-spacer { flex:1; }
.dialog-link { padding:0; border:0; background:transparent; font:inherit; font-size:12px; color:var(--desk-accent-fg,#3b5bdb); cursor:pointer; }
.dialog-link:disabled { opacity:.45; cursor:default; }
fieldset { padding:0; border:0; min-width:0; margin:0; }
input:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:1px; }
.error { color:var(--desk-danger,#b91c1c); overflow-wrap:anywhere; }
.status-ok { margin-top:8px; }
</style>
