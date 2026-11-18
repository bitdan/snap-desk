<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check, Plus, Zap } from 'lucide-vue-next'
import { api } from '@/services/api'
import { updateSettings } from '@/features/settings/settings-write'
import { defaultSettings } from '@/features/settings/settings'
import { cloudFirst, connectionMode, maxServiceKeys, servicePresetId, servicePresets, paddleJobsEndpoint, paddleModels, isPaddleJobsEndpoint, supportsOcr, supportsTranslation, validateService, type RecognitionService } from '../services'
import { t, te } from '@/shared/i18n/i18n'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import ConfirmDialog from '@/shared/ui/ConfirmDialog.vue'
import { brandIcons } from '@/shared/ui/brand-icons'
import { macPlatform } from '@/shared/platform'
import ResourceSettings from '@/features/resources/components/ResourceSettings.vue'
import LocalTranslationModels from './LocalTranslationModels.vue'
import { providerIcon } from '../provider-icons'

/** 本地引擎与在线服务都在此配置、测试和删除；识别、翻译页只负责选择。 */
const settings = ref(defaultSettings()), ready = ref(false), busy = ref(false), error = ref(''), message = ref('')
const draft = ref<RecognitionService | null>(null), preset = ref('groq')
// 已保存密钥只显示掩码；新填写的密钥在保存服务时逐个写入系统凭据。
const keyInfos = ref<{ id: string; hint: string | null }[]>([]), newSecrets = ref<string[]>([''])
const customPaddleEndpoint = ref('')
const paddleEndpointChoice = computed(() => draft.value?.endpoint === paddleJobsEndpoint ? 'official' : 'custom')
const paddleModelOptions = computed(() => (isPaddleJobsEndpoint(draft.value?.endpoint ?? '') ? [...paddleModels] : ['PP-OCRv6', 'PP-OCRv5']).map(value => ({ value, label: value })))
function selectPaddleEndpoint(value: string) {
  if (!draft.value) return
  if (draft.value.endpoint !== paddleJobsEndpoint) customPaddleEndpoint.value = draft.value.endpoint
  draft.value.endpoint = value === 'official' ? paddleJobsEndpoint : customPaddleEndpoint.value
  endpointChanged()
}
const uses = (service: Pick<RecognitionService, 'protocol' | 'vision'>) => [supportsOcr(service as RecognitionService) && t('文字识别'), supportsTranslation(service as RecognitionService) && t('翻译')].filter(Boolean).join(' · ')
// 已添加的服务列表：只显示真实连接，每个连接通过“管理”编辑、测试和删除。
const directory = computed(() => cloudFirst(settings.value.recognitionServices.map(service => {
  const presetId = servicePresetId(service)
  const configured = statuses.value[service.id]
  const status = configured === true ? service.keys.length > 1 ? t('{0} 个密钥', { 0: service.keys.length }) : t('已配置密钥') : configured === false ? t('未配置密钥') : t('凭据状态未读取')
  const { icon, mono, tile } = providerIcon(`service:${service.id}`, service.name, presetId, service.endpoint)
  return { value: `service:${service.id}`, label: service.name, description: service.model, connection: connectionMode(service.endpoint), service, mono, tile, icon, status: [status, active(service.id) ? t('当前使用') : '', uses(service)].filter(Boolean).join(' · '), state: configured === true ? 'ready' : 'idle' }
})))
// 添加服务：按用途分组列出预设，点选后打开对应表单；同一平台可添加多个账号。
const pickerOpen = ref(false)
const presetLabel = (id: string, name: string) => id === 'custom' ? t('自定义兼容服务') : id === 'paddle-ocr' ? 'Paddle API' : id === 'deepl' ? 'DeepL' : name
const presetGroups = computed(() => {
  const tiles = servicePresets.map(item => {
    const { icon, mono, tile } = providerIcon(`preset:${item.id}`, item.name, item.id, item.endpoint)
    return { id: item.id, label: presetLabel(item.id, item.name), icon, mono, tile, added: settings.value.recognitionServices.some(service => servicePresetId(service) === item.id), ocr: supportsOcr(item as unknown as RecognitionService), translation: supportsTranslation(item as unknown as RecognitionService) }
  })
  return [
    { key: 'both', title: t('文字识别与翻译'), hint: t('支持图片输入的模型'), items: tiles.filter(item => item.id !== 'custom' && item.ocr && item.translation) },
    { key: 'ocr', title: t('文字识别'), hint: t('在线 OCR 接口'), items: tiles.filter(item => item.ocr && !item.translation) },
    { key: 'translation', title: t('翻译'), hint: t('文本翻译接口'), items: tiles.filter(item => item.id !== 'custom' && !item.ocr && item.translation) },
    { key: 'custom', title: t('自定义'), hint: t('OpenAI 兼容接口或本机模型'), items: tiles.filter(item => item.id === 'custom') },
  ].filter(group => group.items.length)
})
function openPicker() {
  if (busy.value || !ready.value || draft.value || settings.value.recognitionServices.length >= 64) return
  pickerOpen.value = true
}
function choosePreset(id: string) {
  pickerOpen.value = false
  preset.value = id; add()
}
// 本地引擎在当前页的独立窗口中管理资源；读取失败显示“状态未读取”，不影响在线服务列表。
type LocalKey = 'ppocr' | 'windows' | 'wechat' | 'ct2'
type LocalState = { resources?: Awaited<ReturnType<typeof api.resourceStatus>> | null; system?: Awaited<ReturnType<typeof api.systemOcrStatus>> | null; wechat?: Awaited<ReturnType<typeof api.wechatOcrStatus>> | null; translation?: Awaited<ReturnType<typeof api.translationModelStatus>> | null }
const localState = ref<LocalState>({})
const managingLocal = ref<LocalKey | ''>(''), localChecking = ref(false), localError = ref('')
async function loadLocal() {
  const read = async <T,>(task: () => Promise<T>) => { try { return await task() } catch { return null } }
  const [resources, system, wechat, translation] = await Promise.all([read(() => api.resourceStatus()), read(() => api.systemOcrStatus()), read(() => api.wechatOcrStatus()), read(() => api.translationModelStatus())])
  if (!disposed) localState.value = { resources, system, wechat, translation }
}
const localEngines = computed(() => {
  const { resources, system, wechat, translation } = localState.value
  const unread = t('状态未读取'), ocr = t('文字识别'), translate = t('翻译')
  const ct2Ready = !!translation?.runtimeReady && !!translation.models?.some(pack => pack.ready)
  return [
    { key: 'ppocr' as const, label: 'PaddleOCR', description: t('PP-OCRv6 · 离线'), mono: 'PP', tile: '#2563a8', icon: brandIcons.baidu, ready: resources?.ocrReady === true, unavailable: false,
      status: [resources == null ? unread : resources.ocrReady ? t('已就绪') : t('未下载识别资源'), ocr] },
    { key: 'windows' as const, label: macPlatform ? 'Apple Vision' : 'Windows OCR', description: t('系统自带 · 离线'), mono: macPlatform ? 'AV' : 'W', tile: macPlatform ? '#1d1d1f' : '#0067b8', icon: macPlatform ? '' : brandIcons.microsoft, ready: !!system?.supported && system.languages.length > 0, unavailable: system?.supported === false,
      status: [system == null ? unread : !system.supported ? t('当前平台不支持') : macPlatform ? t('支持 {0} 种识别语言', { 0: system.languages.length }) : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装语言包'), ocr] },
    { key: 'wechat' as const, label: 'WeChat OCR', description: t('实验性 · 离线'), mono: 'WX', tile: '#07a35a', icon: '', ready: wechat?.detected === true, unavailable: wechat?.supported === false,
      status: [wechat == null ? unread : wechat.supported === false ? t('当前平台不支持') : wechat.detected ? t('已找到 WeChat OCR 组件') : t('未找到 WeChat OCR 组件'), ocr] },
    { key: 'ct2' as const, label: 'CTranslate2', description: t('本地中英互译 · 离线'), mono: 'CT', tile: '#475569', icon: '', ready: ct2Ready, unavailable: false,
      status: [translation == null ? unread : ct2Ready ? t('已安装') : t('未安装'), translate] },
  ].map(item => ({ ...item, status: item.status.join(' · ') }))
})
const managedEngine = computed(() => localEngines.value.find(item => item.key === managingLocal.value))
function closeLocal() { managingLocal.value = ''; localError.value = ''; void loadLocal() }
async function recheckLocal(kind: 'windows' | 'wechat') {
  if (localChecking.value) return
  localChecking.value = true; localError.value = ''
  try {
    if (kind === 'windows') { const system = await api.systemOcrStatus(); if (!disposed) localState.value = { ...localState.value, system } }
    else { const wechat = await api.wechatOcrStatus(); if (!disposed) localState.value = { ...localState.value, wechat } }
  } catch (e) { if (!disposed) localError.value = String(e) }
  finally { if (!disposed) localChecking.value = false }
}
onMounted(loadLocal)
const draftIcon = computed(() => providerIcon(draft.value ? `preset:${preset.value}` : '', draft.value?.name ?? '', preset.value, draft.value?.endpoint))
const draftSaved = computed(() => !!draft.value && settings.value.recognitionServices.some(item => item.id === draft.value!.id))
const dialogTitle = computed(() => !draft.value ? '' : draftSaved.value ? t('管理服务') : t('添加服务'))
function closeDialog() {
  if (busy.value) return
  cancel()
}
const statuses = ref<Record<string, boolean | null>>({})
let disposed = false, revision = 0
async function load() {
  error.value = ''
  try {
    const value = await api.status()
    if (disposed) return
    settings.value = value.settings; ready.value = true
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
onUnmounted(() => { disposed = true; revision++; newSecrets.value = [] })
function add() {
  customPaddleEndpoint.value = ''
  if (busy.value || !ready.value) return
  const selected = servicePresets.find(item => item.id === preset.value)!
  draft.value = { id: crypto.randomUUID(), name: selected.id === 'custom' ? t('自定义兼容服务') : selected.name, protocol: selected.protocol, endpoint: selected.endpoint, model: selected.model, vision: selected.vision, keys: [] }
  keyInfos.value = []; newSecrets.value = ['']; message.value = ''; error.value = ''; testResult.value = ''; keyTests.value = {}; revision++
}
function edit(service: RecognitionService) {
  if (busy.value || !ready.value) return
  customPaddleEndpoint.value = service.endpoint === paddleJobsEndpoint ? '' : service.endpoint
  preset.value = servicePresetId(service); draft.value = { ...service, keys: [...service.keys] }
  keyInfos.value = service.keys.map(id => ({ id, hint: null })); newSecrets.value = []; message.value = ''; error.value = ''; testResult.value = ''; keyTests.value = {}
  void loadKeys(service.id, ++revision)
}
async function loadKeys(id: string, version: number) {
  try {
    const keys = await api.recognitionServiceKeys(id)
    if (!disposed && version === revision) { keyInfos.value = keys; if (!keys.length) newSecrets.value = [''] }
  } catch (e) { if (!disposed && version === revision) error.value = String(e) }
}
// 修改地址后原密钥不会发往新地址：已保存的密钥在保存时一并清除，需要重新填写。
const savedService = computed(() => settings.value.recognitionServices.find(item => item.id === draft.value?.id))
const endpointMoved = computed(() => !!savedService.value && savedService.value.endpoint !== draft.value?.endpoint)
const visibleKeys = computed(() => endpointMoved.value ? [] : keyInfos.value)
function addKeyInput() { if (visibleKeys.value.length + newSecrets.value.length < maxServiceKeys) newSecrets.value.push('') }
async function removeKey(id: string) {
  const service = savedService.value
  if (!service || busy.value) return
  busy.value = true; error.value = ''; testResult.value = ''
  try {
    // 先删凭据再从配置移除标识；失败时标识仍在，可重试。
    await api.recognitionServiceCredential(service.id, service.endpoint, id, null)
    const next = { ...service, keys: service.keys.filter(key => key !== id) }
    const saved = await updateSettings(current => ({ ...current, recognitionServices: current.recognitionServices.map(item => item.id === service.id ? next : item) }))
    if (!disposed) {
      settings.value = saved; keyInfos.value = keyInfos.value.filter(item => item.id !== id)
      if (draft.value) draft.value.keys = draft.value.keys.filter(key => key !== id)
      if (!keyInfos.value.length && !newSecrets.value.length) newSecrets.value = ['']
      statuses.value[service.id] = await api.recognitionServiceStatus(service.id)
      testResult.value = '密钥已删除。'
    }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
function cancel() { draft.value = null; keyInfos.value = []; newSecrets.value = ['']; testResult.value = ''; keyTests.value = {}; revision++ }
function endpointChanged() {
  revision++
  if (endpointMoved.value && !newSecrets.value.length) newSecrets.value = ['']
  if (draft.value?.protocol === 'paddle-ocr' && !isPaddleJobsEndpoint(draft.value.endpoint) && !['PP-OCRv5', 'PP-OCRv6'].includes(draft.value.model)) draft.value.model = 'PP-OCRv6'
}
function active(id: string) { return (settings.value.ocr.provider === 'service' && settings.value.ocr.serviceId === id) || (settings.value.ai.provider !== 'local-ctranslate2' && settings.value.ai.serviceId === id) }
async function save() {
  if (!draft.value || busy.value) return
  const previous = savedService.value, moved = endpointMoved.value, version = revision
  const added = newSecrets.value.filter(Boolean).map(secret => ({ id: `k-${crypto.randomUUID().slice(0, 8)}`, secret }))
  const service = { ...draft.value, keys: [...(moved ? [] : draft.value.keys), ...added.map(item => item.id)] }
  const invalid = validateService(service)
  if (invalid) { error.value = invalid; return }
  if (added.some(item => item.secret.length > 2048 || /\p{Cc}/u.test(item.secret))) { error.value = '密钥过长或含控制字符。'; return }
  busy.value = true; error.value = ''; message.value = ''
  try {
    // 地址变更时先按旧地址清除原密钥，避免凭据库中残留无法管理的条目。
    if (previous && moved) for (const key of previous.keys) await api.recognitionServiceCredential(previous.id, previous.endpoint, key, null)
    const saved = await updateSettings(current => ({ ...current, recognitionServices: current.recognitionServices.some(item => item.id === service.id) ? current.recognitionServices.map(item => item.id === service.id ? service : item) : [...current.recognitionServices, service] }))
    if (!disposed) settings.value = saved
    for (const item of added) await api.recognitionServiceCredential(service.id, service.endpoint, item.id, item.secret)
    const configured = await api.recognitionServiceStatus(service.id)
    if (!disposed && version === revision) { statuses.value[service.id] = configured; cancel(); message.value = '服务已保存。' }
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
// 测试使用已保存的连接与密钥：翻译服务发送 Hello，仅 OCR 的服务上传一张空白小图。
const testing = ref(false), testResult = ref('')
async function testService() {
  if (!draft.value || !draftSaved.value || busy.value || testing.value) return
  testing.value = true; error.value = ''; testResult.value = ''
  try { await api.recognitionServiceTest(draft.value.id); if (!disposed) testResult.value = '测试成功，服务可用。' }
  catch (e) { if (!disposed) error.value = String(e) }
  finally { if (!disposed) testing.value = false }
}
// 逐个密钥测试：只用该密钥请求、不回退，结果显示在对应行上。
const keyTests = ref<Record<string, { state: 'testing' | 'ok' | 'error', message?: string }>>({})
async function testKey(id: string) {
  if (!draft.value || !draftSaved.value || busy.value || testing.value || keyTests.value[id]?.state === 'testing') return
  const serviceId = draft.value.id, version = revision
  keyTests.value = { ...keyTests.value, [id]: { state: 'testing' } }
  let result: { state: 'ok' | 'error', message?: string }
  try { await api.recognitionServiceTest(serviceId, id); result = { state: 'ok' } }
  catch (e) { result = { state: 'error', message: String(e) } }
  if (!disposed && version === revision) keyTests.value = { ...keyTests.value, [id]: result }
}
// 删除前统一确认；正在使用的连接一并解除绑定，OCR 回到 PaddleOCR、翻译回到本地 CTranslate2。
const pendingRemove = ref<RecognitionService | null>(null), removeError = ref('')
const removeDescription = computed(() => {
  const service = pendingRemove.value
  if (!service) return ''
  const ocr = settings.value.ocr.provider === 'service' && settings.value.ocr.serviceId === service.id
  const translation = settings.value.ai.provider !== 'local-ctranslate2' && settings.value.ai.serviceId === service.id
  return [t('删除「{0}」及其已保存的密钥？', { 0: service.name }), ocr && t('删除后 OCR 将改用 PaddleOCR。'), translation && t('删除后翻译将改用本地 CTranslate2。'), t('此操作不可撤销。')].filter(Boolean).join(' ')
})
function requestRemoveDraft() {
  const service = settings.value.recognitionServices.find(item => item.id === draft.value?.id)
  if (service && !busy.value) { removeError.value = ''; pendingRemove.value = service }
}
function updateRemoveOpen(open: boolean) { if (!open && !busy.value) pendingRemove.value = null }
async function remove() {
  const service = pendingRemove.value
  if (!service || busy.value) return
  busy.value = true; error.value = ''; removeError.value = ''; message.value = ''
  try {
    // 先删除凭据；失败时保留连接，以便用户重试而不留下不可管理的密钥。
    for (const key of service.keys) await api.recognitionServiceCredential(service.id, service.endpoint, key, null)
    const saved = await updateSettings(current => ({
      ...current,
      recognitionServices: current.recognitionServices.filter(item => item.id !== service.id),
      ocr: current.ocr.provider === 'service' && current.ocr.serviceId === service.id ? { ...current.ocr, provider: 'local-ppocr', serviceId: '' } : current.ocr,
      ai: current.ai.serviceId === service.id ? { ...current.ai, provider: 'local-ctranslate2', serviceId: '' } : current.ai,
    }))
    if (!disposed) {
      settings.value = saved; pendingRemove.value = null
      if (draft.value?.id === service.id) cancel()
      message.value = '服务已删除。'
    }
  } catch (e) { if (!disposed) removeError.value = String(e) }
  finally { if (!disposed) busy.value = false }
}
async function portal() {
  try { await api.openRecognitionPortal(preset.value) } catch (e) { error.value = String(e) }
}
</script>

<template>
  <section class="service-settings service-directory-page">
    <h2 class="section-heading">{{ t('本地引擎') }}</h2>
    <div class="settings-card service-directory">
      <div v-for="item in localEngines" :key="item.key" class="service-item">
        <span class="service-mono" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" class="service-logo" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
        <span class="service-text">
          <span class="service-name">{{ item.label }}<small>{{ item.description }}</small></span>
          <span class="service-status" role="status"><i :class="item.ready ? 'ready' : 'idle'" aria-hidden="true" />{{ item.status }}</span>
        </span>
        <button type="button" class="service-action outlined" :data-local-engine="item.key" :disabled="item.unavailable" @click="managingLocal = item.key">{{ t('管理') }}</button>
      </div>
    </div>
    <h2 class="section-heading">{{ t('在线服务') }}<button type="button" data-add-service :disabled="busy || !ready || !!draft || settings.recognitionServices.length >= 64" @click="openPicker">{{ t('添加服务') }}</button></h2>
    <p v-if="!ready" role="status" class="hint">{{ t('正在读取配置…') }}</p>
    <!-- 只列出已添加的连接；新增从预设面板选择，编辑、测试与删除在独立窗口中进行。 -->
    <div class="settings-card service-directory">
      <div v-for="item in directory" :key="item.value" class="service-item" :class="{ selected: draft?.id === item.service.id }">
        <span class="service-mono" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" class="service-logo" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
        <span class="service-text">
          <span class="service-name">{{ item.label }}<small v-if="item.description">{{ item.description }}</small></span>
          <span class="service-status" role="status"><i :class="item.state" aria-hidden="true" />{{ item.status }}</span>
        </span>
        <button type="button" class="service-action outlined" :data-provider="item.value" :disabled="busy || !ready" @click="edit(item.service)">{{ t('管理') }}</button>
      </div>
      <button v-if="ready && !directory.length" type="button" class="service-empty" :disabled="busy" @click="openPicker"><Plus :size="14" aria-hidden="true" />{{ t('尚未添加在线服务，点击选择要接入的服务') }}</button>
    </div>
    <DetachedWindow :open="pickerOpen" :title="t('添加服务')" :width="560" :height="440" @close="pickerOpen = false">
      <div v-if="pickerOpen" class="app-shell dialog-shell">
        <section class="service-dialog" role="dialog" :aria-label="t('添加服务')">
          <header class="service-dialog-header"><h2>{{ t('添加服务') }}</h2></header>
          <div v-for="group in presetGroups" :key="group.key" class="preset-group">
            <h3>{{ group.title }}<small>{{ group.hint }}</small></h3>
            <div class="preset-grid">
              <button v-for="item in group.items" :key="item.id" type="button" class="preset-tile" :data-preset="item.id" @click="choosePreset(item.id)">
                <span class="service-mono" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" class="service-logo" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
                <span class="preset-name">{{ item.label }}</span>
                <i v-if="item.added" class="preset-added" :title="t('已添加')" aria-hidden="true" />
              </button>
            </div>
          </div>
          <p class="section-note">{{ t('同一平台可添加多个账号；添加后在“文字识别”“翻译”页选择使用。') }}</p>
        </section>
      </div>
    </DetachedWindow>
    <DetachedWindow :open="!!managingLocal" :title="managedEngine?.label ?? ''" :width="560" :height="420" @close="closeLocal">
      <div v-if="managedEngine" class="app-shell dialog-shell">
        <section class="service-dialog" role="dialog" :aria-label="managedEngine.label">
          <header class="service-dialog-header">
            <span class="service-mono" :class="{ logo: managedEngine.icon }" :style="managedEngine.icon ? undefined : { background: managedEngine.tile }" aria-hidden="true"><span v-if="managedEngine.icon" class="service-logo" v-html="managedEngine.icon" /><template v-else>{{ managedEngine.mono }}</template></span>
            <h2>{{ managedEngine.label }}</h2>
          </header>
          <ResourceSettings v-if="managedEngine.key === 'ppocr'" kind="ocr" />
          <LocalTranslationModels v-else-if="managedEngine.key === 'ct2'" @changed="localState = { ...localState, translation: $event }" />
          <div v-else-if="managedEngine.key === 'windows'" class="local-detail">
            <p role="status">{{ managedEngine.status }}</p>
            <ul v-if="localState.system?.languages.length" class="language-list"><li v-for="item in localState.system.languages" :key="item.tag">{{ item.name }} <small>{{ item.tag }}</small></li></ul>
            <p v-if="!macPlatform" class="section-note">{{ t('语言包由 Windows 管理，可在 Windows 设置的“语言和区域”中添加。') }}</p>
            <Button variant="outline" :disabled="localChecking" @click="recheckLocal('windows')">{{ localChecking ? t('正在检查…') : t('刷新语言包') }}</Button>
          </div>
          <div v-else class="local-detail">
            <p role="status">{{ localState.wechat?.error ? te(localState.wechat.error) : managedEngine.status }}</p>
            <Button variant="outline" :disabled="localChecking" @click="recheckLocal('wechat')">{{ localChecking ? t('正在检测 WeChat OCR…') : t('重新检测') }}</Button>
          </div>
          <p v-if="localError" class="notice error" role="alert">{{ te(localError) }}</p>
        </section>
      </div>
    </DetachedWindow>
    <DetachedWindow :open="!!draft" :title="dialogTitle" :width="500" :height="500" :busy="busy" @close="closeDialog">
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
              <label v-else-if="draft.protocol === 'deepl'" class="setting-row dialog-field"><span>{{ t('完整接口地址') }}</span><SelectControl v-model="draft.endpoint" :aria-label="t('服务地址')" :options="[{ value: 'https://api-free.deepl.com/v2/translate', label: 'DeepL API Free' }, { value: 'https://api.deepl.com/v2/translate', label: 'DeepL API Pro' }]" @update:model-value="endpointChanged" /></label>
              <label v-else class="setting-row dialog-field"><span>{{ t('完整接口地址') }}</span><input v-model.trim="draft.endpoint" type="url" autocomplete="off" @input="endpointChanged" /></label>
              <label v-if="draft.protocol !== 'deepl' && draft.protocol !== 'paddle-ocr'" class="setting-row dialog-field"><span>{{ draft.protocol === 'ocr-space' ? t('OCR.space 引擎（1 / 2 / 3）') : t('模型名称') }}</span><input v-model.trim="draft.model" maxlength="200" /></label>
              <div class="setting-row dialog-field key-field"><span>API Key / Token</span>
                <div class="key-list">
                  <div v-for="(item, index) in visibleKeys" :key="item.id" class="key-row" data-key-row>
                    <code>{{ item.hint ?? t('未找到密钥') }}</code><small v-if="index === 0 && visibleKeys.length + newSecrets.length > 1">{{ t('首选') }}</small>
                    <span class="actions-spacer" />
                    <span v-if="keyTests[item.id]?.state === 'ok'" class="key-test ok" role="status"><Check :size="12" aria-hidden="true" />{{ t('可用') }}</span>
                    <span v-else-if="keyTests[item.id]?.state === 'error'" class="key-test error" role="alert" :title="te(keyTests[item.id].message ?? '')">{{ t('不可用') }}</span>
                    <button v-if="draftSaved && !endpointMoved" type="button" class="dialog-link" data-key-test :disabled="busy || testing || keyTests[item.id]?.state === 'testing'" :aria-label="t('测试此密钥')" :title="t('仅用此密钥发送一次测试请求，可能产生少量费用。')" @click="testKey(item.id)">{{ keyTests[item.id]?.state === 'testing' ? t('正在测试…') : t('测试') }}</button>
                    <button type="button" class="dialog-link danger" :disabled="busy" :aria-label="t('删除此密钥')" @click="removeKey(item.id)">{{ t('删除') }}</button>
                  </div>
                  <input v-for="(_, index) in newSecrets" :key="`new-${index}`" v-model="newSecrets[index]" type="password" autocomplete="new-password" :aria-label="t('新密钥')" :placeholder="t('填写 API Key / Token')" />
                  <button type="button" class="dialog-link add-key" :disabled="busy || visibleKeys.length + newSecrets.length >= maxServiceKeys" @click="addKeyInput"><Plus :size="12" aria-hidden="true" />{{ t('添加另一个密钥') }}</button>
                </div>
              </div>
              <label v-if="draft.protocol === 'openai-compatible'" class="setting-row dialog-switch"><span>{{ t('此模型支持图片输入，可用于 OCR') }}</span><input v-model="draft.vision" class="mini-switch" type="checkbox" role="switch" /></label>
            </fieldset>
            <p v-if="connectionMode(draft.endpoint) === 'online'" class="section-note">{{ t('用途：{0}', { 0: uses(draft) }) }} · {{ supportsOcr(draft) ? t('OCR 会将选区图片发送到此服务，可能产生费用。') : t('在线翻译会发送原文，可能产生费用。') }}</p>
            <p v-if="visibleKeys.length + newSecrets.filter(Boolean).length > 1" class="section-note">{{ t('请求先用第一个密钥，认证失败或额度用尽时依次换下一个。') }}</p>
            <p v-if="endpointMoved && keyInfos.length" class="section-note">{{ t('地址已修改，保存后原有密钥会被清除，请重新填写。') }}</p>
            <p v-if="testResult" class="status-ok" role="status"><Check :size="14" />{{ t(testResult) }}</p>
            <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
            <div class="actions">
              <button v-if="preset !== 'custom'" type="button" class="dialog-link" :title="t('当前通过 API Key / Token 接入；浏览器登录用于获取凭据，不会自动授权 Snap Desk。')" @click="portal">{{ t('获取 API Key / Token') }}</button>
              <button v-if="draftSaved" type="button" class="dialog-link danger" :disabled="busy" @click="requestRemoveDraft">{{ t('删除服务') }}</button>
              <span class="actions-spacer" />
              <Button type="button" variant="outline" data-service-test :disabled="!draftSaved || endpointMoved || busy || testing" :title="draftSaved && !endpointMoved ? t('使用已保存的配置发送一次测试请求，可能产生少量费用。') : t('保存后可测试。')" @click="testService"><Zap :size="14" aria-hidden="true" />{{ testing ? t('正在测试…') : t('测试连接') }}</Button>
              <Button type="button" variant="outline" :disabled="busy" @click="closeDialog">{{ t('取消') }}</Button>
              <Button type="submit" :disabled="busy">{{ busy ? t('正在保存…') : t('保存服务') }}</Button>
            </div>
          </form>
        </section>
      </div>
    </DetachedWindow>
    <ConfirmDialog :open="!!pendingRemove" :title="t('删除服务')" :description="removeDescription" :confirm-label="t('确认删除')" :busy="busy" destructive :error="te(removeError)" @update:open="updateRemoveOpen" @confirm="remove" />
    <p class="section-note">{{ t('密钥保存在系统凭据中，不随配置导出。') }}</p>
    <p v-if="message" class="status-ok" role="status"><Check :size="14" />{{ t(message) }}</p>
    <p v-if="error && !draft" class="error" role="alert">{{ te(error) }}<Button v-if="!ready" variant="ghost" @click="load">{{ t('重试') }}</Button></p>
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
.dialog-link.danger { color:var(--desk-danger,#b91c1c); }
.dialog-link:disabled { opacity:.45; cursor:default; }
fieldset { padding:0; border:0; min-width:0; margin:0; }
input:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:1px; }
.error { color:var(--desk-danger,#b91c1c); overflow-wrap:anywhere; }
.status-ok { margin-top:8px; }
.service-empty { display:flex; align-items:center; gap:6px; width:100%; min-height:48px; padding:8px 12px; border:0; background:transparent; color:var(--desk-muted,#5f6670); font:inherit; font-size:13px; text-align:left; cursor:pointer; }
.service-empty:hover:enabled { color:var(--desk-accent-fg,#3b5bdb); }
.preset-group { margin-bottom:14px; }
.preset-group h3 { display:flex; align-items:baseline; gap:8px; margin:0 0 6px; font-size:13px; font-weight:600; }
.preset-group h3 small { font-size:11px; font-weight:400; color:var(--desk-muted,#5f6670); }
.preset-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(150px,1fr)); gap:4px; }
.preset-tile { display:flex; align-items:center; gap:8px; min-height:36px; padding:4px 8px; border:0; border-radius:6px; background:transparent; color:var(--desk-text,#1f2328); font:inherit; font-size:13px; text-align:left; cursor:pointer; }
.preset-tile:hover { background:var(--desk-hover,rgba(127,127,127,.12)); }
.preset-tile:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:-2px; }
.preset-tile .service-mono { width:24px; height:24px; }
.preset-name { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.preset-added { width:6px; height:6px; flex:none; border-radius:50%; background:var(--ui-success-fg,#24834b); }
.local-detail { display:flex; flex-direction:column; align-items:flex-start; gap:8px; }
.local-detail p { margin:0; }
.language-list { margin:0; padding-left:18px; line-height:1.8; }
.language-list small { color:var(--desk-muted,#5f6670); }
.notice.error { margin-top:8px; }
.key-field { align-items:flex-start; }
.key-field > span { padding-top:5px; }
.key-list { flex:1; min-width:0; display:flex; flex-direction:column; gap:6px; }
.key-test { display:inline-flex; align-items:center; gap:2px; font-size:12px; }
.key-test.ok { color:var(--desk-success,#15803d); }
.key-test.error { color:var(--desk-danger,#b91c1c); cursor:help; }
.key-row { display:flex; align-items:center; gap:8px; min-height:28px; padding:0 8px; border-radius:6px; background:var(--desk-soft,#f2f4f7); }
.key-row code { font-size:12px; }
.key-row small { font-size:11px; color:var(--desk-muted,#5f6670); }
.add-key { display:inline-flex; align-items:center; gap:4px; align-self:flex-start; }
.status-ok { display:flex; align-items:center; gap:4px; color:var(--ui-success-fg,#24834b); }
</style>
