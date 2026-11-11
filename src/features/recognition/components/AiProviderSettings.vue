<script lang="ts">
// 只缓存公开的资源状态，不保存密钥或未提交的设置草稿。
let latestLocalStatus: import('@/features/recognition/ai').LocalTranslationStatus | null = null
let localStatusRevision = 0
</script>

<script setup lang="ts">
import ProviderMenu, { type ProviderMenuItem } from './ProviderMenu.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { updateSettings } from '@/features/settings/settings-write'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import { deepLEndpoints, defaultAiSettings, targetLanguages, translationPresets, validateAiSettings } from '@/features/recognition/ai'
import type { AiSettings, LocalTranslationStatus } from '@/features/recognition/ai'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import InfoTip from '@/shared/ui/InfoTip.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'
import { cloudFirst, connectionMode, supportsTranslation, translationPresetId, translationService, type RecognitionService } from '@/features/recognition/services'

const activeSettings = ref<AiSettings | null>(null)
const services = ref<RecognitionService[]>([])
const draft = ref(defaultAiSettings()), secret = ref(''), configured = ref<boolean | null>(null)
const localStatus = ref<LocalTranslationStatus | null>(latestLocalStatus)
const checking = ref(false)
const preset = ref('custom')
/** 按当前界面的显示名称排序，切换语言后同步更新，保留平台内部标识。 */
const serviceOptions = computed(() => [
  { value: 'custom', label: t('自定义') },
  ...translationPresets.map(item => ({ value: item.value, label: t(item.label) })),
  { value: 'deepl', label: 'DeepL' },
].sort((left, right) => left.label.localeCompare(right.label, 'zh-CN', { sensitivity: 'base' })))
let disposed = false, credentialStatusRevision = 0
const localEndpoint = computed(() => { try { return ['localhost', '127.0.0.1', '[::1]'].includes(new URL(draft.value.endpoint).hostname) } catch { return false } })
const local = computed(() => draft.value.provider === 'local-ctranslate2')
const languages = computed(() => local.value ? targetLanguages.slice(0, 2) : targetLanguages)
watch(local, value => { if (value && !['zh-CN', 'en'].includes(draft.value.targetLanguage)) draft.value.targetLanguage = 'zh-CN' })
// 地址切换立即清空未保存密钥，避免把上一服务的输入写入新服务。
watch(() => [draft.value.endpoint, draft.value.provider, draft.value.serviceId], () => {
  credentialStatusRevision++; secret.value = ''; configured.value = null
}, { flush: 'sync' })
const busy = ref(false), ready = ref(false), error = ref(''), message = ref('')
async function loadSettings() {
  try {
    const settings = await api.status()
    if (disposed) return
    services.value = settings.settings.recognitionServices
    const saved = settings.settings.ai
    const service = services.value.find(item => item.id === saved.serviceId)
    draft.value = service && saved.provider !== 'local-ctranslate2' ? translationService(saved, service) : { ...saved }; ready.value = true
    activeSettings.value = { ...draft.value }
    syncPreset()
    if (!local.value) {
      const revision = credentialStatusRevision
      const value = await api.aiProviderStatus()
      if (!disposed && revision === credentialStatusRevision) configured.value = value
    }
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
async function refreshLocalStatus() {
  const revision = ++localStatusRevision
  checking.value = true
  try {
    const next = await api.translationModelStatus()
    if (revision === localStatusRevision) {
      latestLocalStatus = next
      if (!disposed) localStatus.value = next
    }
  } catch (e) {
    if (revision === localStatusRevision) {
      latestLocalStatus = null
      if (!disposed) { localStatus.value = null; error.value = String(e) }
    }
  } finally { if (!disposed) checking.value = false }
}
onMounted(() => {
  // 配置/凭证与引擎校验独立返回，慢校验或失败不能拖住整个表单。
  void loadSettings()
  void refreshLocalStatus()
})
onUnmounted(() => { disposed = true })
/** 只覆盖 AI 字段，读取最新配置以保留其他页面已保存的设置。密钥从不进入 settings。 */
const autosave = useAutoSave(async () => {
  if (!ready.value) return
  const ai = { ...draft.value }, key = secret.value, revision = credentialStatusRevision
  const saved = await updateSettings(current => ({ ...current, ai }))
  if (!disposed) activeSettings.value = { ...saved.ai }
  if (ai.provider !== 'local-ctranslate2' && key) {
    await api.aiCredentialSet(key, ai.endpoint)
    // 旧服务写入结束时，不能清空用户已为新服务重新输入的同一段密钥。
    if (revision === credentialStatusRevision && secret.value === key) secret.value = ''
  }
  const status = ai.provider === 'local-ctranslate2' ? false : await api.aiProviderStatus()
  if (!disposed && revision === credentialStatusRevision && ai.endpoint === draft.value.endpoint && ai.provider === draft.value.provider) configured.value = status
}, () => validateAiSettings(draft.value))
/** 密钥在结束编辑后保存，避免逐字符写入系统凭据库。 */
function syncPreset() {
  preset.value = draft.value.provider === 'deepl' ? 'deepl' : draft.value.provider === 'online-llm'
    ? translationPresets.find(item => item.endpoint === draft.value.endpoint)?.value ?? 'custom' : 'custom'
}
/** 界面只区分本地/在线；平台选择映射回原有协议，兼容已保存的 DeepL 配置。 */
function modeChanged(mode: string) {
  draft.value.serviceId = ''
  draft.value.provider = mode === 'local' ? 'local-ctranslate2'
    : deepLEndpoints.includes(draft.value.endpoint) ? 'deepl' : 'online-llm'
  secret.value = ''; configured.value = null; message.value = ''; error.value = ''
  syncPreset()
  autosave.commit()
}
function selectService(id: string) {
  const service = services.value.find(item => item.id === id)
  if (service) draft.value = translationService(draft.value, service)
  else draft.value.serviceId = ''
  secret.value = ''; configured.value = null
  autosave.commit()
}
function applyPreset() {
  draft.value.serviceId = ''
  secret.value = ''; configured.value = null
  if (preset.value === 'deepl') {
    draft.value.provider = 'deepl'
    if (!deepLEndpoints.includes(draft.value.endpoint)) draft.value.endpoint = deepLEndpoints[0]!
    draft.value.model = ''
  } else {
    const wasDeepL = draft.value.provider === 'deepl'
    draft.value.provider = 'online-llm'
    const selected = translationPresets.find(item => item.value === preset.value)
    if (selected) { draft.value.endpoint = selected.endpoint; draft.value.model = selected.model }
    else if (wasDeepL) { draft.value.endpoint = ''; draft.value.model = '' }
  }
  autosave.commit()
}
/** 二级菜单同时列出离线引擎、平台预设和已保存账号，不再逐层打开下拉框。 */
const selectedMenu = computed(() => local.value ? 'offline' : draft.value.serviceId ? `service:${draft.value.serviceId}` : `preset:${preset.value}`)
const providerOptions = computed(() => [{ value: 'offline', label: 'CTranslate2', connection: 'offline' as const }, ...serviceOptions.value.map(item => ({ ...item, label: item.value === 'local' ? t('本机模型') : item.label, value: `preset:${item.value}`, sortPriority: item.value === 'custom' ? 1 : undefined, connection: item.value === 'custom' ? (preset.value === 'custom' && !draft.value.serviceId ? connectionMode(draft.value.endpoint) : undefined) : item.value === 'local' ? 'local-api' as const : 'online' as const }))])
const savedOptions = computed(() => services.value.filter(supportsTranslation).map(item => ({ value: `service:${item.id}`, label: item.name, description: item.model || item.protocol, connection: connectionMode(item.endpoint), presetId: translationPresetId(item) })))
const activeMenu = computed(() => {
  const value = activeSettings.value
  if (!value) return ''
  if (value.provider === 'local-ctranslate2') return 'offline'
  if (value.serviceId) return `service:${value.serviceId}`
  return `preset:${value.provider === 'deepl' ? 'deepl' : translationPresets.find(item => item.endpoint === value.endpoint)?.value ?? 'custom'}`
})
const menuOptions = computed(() => {
  const options: ProviderMenuItem[] = []
  for (const item of providerOptions.value) {
    const saved = savedOptions.value.filter(service => `preset:${service.presetId}` === item.value)
    if (!saved.length) options.push(item)
    else {
      // 仍在使用旧版直接配置时标明来源，避免把真实生效配置静默映射到另一账号。
      if (activeMenu.value === item.value) options.push({ ...item, label: `${item.label} · ${t('旧配置')}` })
      options.push(...saved)
    }
  }
  return cloudFirst(options).map(item => ({ ...item, active: item.value === activeMenu.value }))
})
const activeLabel = computed(() => {
  const item = menuOptions.value.find(item => item.value === activeMenu.value)
  return [item?.label, activeSettings.value?.provider !== 'local-ctranslate2' ? activeSettings.value?.model : ''].filter(Boolean).join(' · ')
})
function selectMenu(value: string) {
  if (busy.value || !ready.value) return
  if (value === 'offline') { modeChanged('local'); return }
  if (value.startsWith('service:')) { selectService(value.slice(8)); return }
  preset.value = value.slice(7); applyPreset()
}
function changed(event: Event) {
  if ((event.target as HTMLInputElement).type !== 'password') autosave.schedule()
}
async function credentialAction(action: 'delete' | 'test') {
  await autosave.flush()
  if (autosave.error.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    if (action === 'delete') { await api.aiCredentialSet(null, draft.value.endpoint); configured.value = false; message.value = '密钥已删除。' }
    else { await api.aiProviderTest(); message.value = '翻译测试成功。' }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 引擎和语言包均由用户主动下载或导入，状态与 OCR 独立。 */
async function modelAction(modelId: string, action: 'download' | 'import' | 'delete') {
  if (busy.value || checking.value) return
  const revision = ++localStatusRevision
  busy.value = true; error.value = ''; message.value = modelId === 'runtime' ? '正在安装引擎…' : ''
  try {
    const next = await api.translationModelAction(modelId, action)
    if (revision === localStatusRevision) latestLocalStatus = next
    if (!disposed) { localStatus.value = next; message.value = modelId === 'runtime' ? '引擎状态已更新。' : '语言包状态已更新。' }
  }
  catch (e) { message.value = ''; error.value = String(e) }
  finally { busy.value = false }
}
</script>

<template>
  <section class="ai-settings recognition-settings-layout">
    <aside class="recognition-settings-sidebar">
      <ProviderMenu :model-value="selectedMenu" :options="menuOptions" :label="t('翻译引擎')" :disabled="busy || !ready" @update:model-value="selectMenu" />
    </aside>
    <section class="recognition-settings-content recognition-section translation-section">
    <!-- 生效变化由读屏播报；视觉状态由菜单勾选统一表达。 -->
    <p class="sr-only" role="status" data-current-translation>{{ activeSettings ? t('当前翻译：{0}', { 0: activeLabel }) : t('正在读取配置…') }}</p>
    <fieldset :disabled="busy || !ready" @input="changed" @change="autosave.commit" @focusout="autosave.flush">

      <div v-if="local" class="local-resources">
        <div class="model-row runtime-row">
          <span class="runtime-status" :class="{ 'status-ok': localStatus?.runtimeReady }" role="status"><Check v-if="localStatus?.runtimeReady" :size="14" />{{ !localStatus ? checking ? t("正在检查…") : t("引擎状态未读取") : localStatus.runtimeReady ? t("本地引擎已安装") : t("翻译引擎未安装或校验失败") }}</span>
          <div class="model-actions">
            <Button variant="outline" :disabled="checking || !localStatus?.runtimeSupported" @click="modelAction('runtime', 'download')">{{ t("下载引擎") }}</Button>
            <Button variant="outline" :disabled="checking || !localStatus?.runtimeSupported" @click="modelAction('runtime', 'import')">{{ t("导入引擎 ZIP") }}</Button>
          </div>
        </div>
        <p v-if="localStatus && !localStatus.runtimeSupported" class="hint">{{ t("翻译引擎下载和导入目前仅支持 Windows x64。") }}</p>
        <div v-for="pack in localStatus?.models" :key="pack.modelId" class="model-row">
          <span class="model-name">{{ pack.modelId.includes('en-zh') ? t("英语 → 简体中文") : t("中文 → 英语") }}</span>
          <span :class="{ 'status-ok': pack.ready }" role="status"><Check v-if="pack.ready" :size="14" />{{ pack.ready ? t("已校验") : t("未安装或校验失败") }}</span>
          <div class="model-actions">
            <Button variant="outline" :disabled="checking || !pack.downloadable" @click="modelAction(pack.modelId, 'download')">{{ t("下载") }}{{ pack.downloadBytes ? `（${Math.ceil(pack.downloadBytes / 1048576)} MiB）` : '' }}</Button>
            <Button variant="outline" :disabled="checking" @click="modelAction(pack.modelId, 'import')">{{ t("导入语言包") }}</Button>
            <Button v-if="pack.revision" variant="ghost" :disabled="checking" @click="modelAction(pack.modelId, 'delete')">{{ t("卸载") }}</Button>
          </div>
        </div>
      </div>
      <div v-else class="online-fields">

        <div v-if="draft.serviceId" class="saved-connection">
          <dl class="connection-summary"><div><dt>{{ t('服务地址') }}</dt><dd>{{ draft.endpoint }}</dd></div><div v-if="draft.model"><dt>{{ t('模型名称') }}</dt><dd>{{ draft.model }}</dd></div></dl>
          <div class="credential-status" role="status"><span :class="{ 'status-ok': configured }"><Check v-if="configured" :size="14" />{{ configured === null ? t('凭据状态未读取') : configured ? t('已配置密钥') : localEndpoint ? t('本机服务可不填密钥') : t('未配置密钥') }}</span><InfoTip :text="t('连接与凭据在服务管理中维护。')" /></div>
        </div>
        <template v-if="!draft.serviceId">

        <label v-if="draft.provider === 'deepl'">{{ t("服务地址") }}<SelectControl v-model="draft.endpoint" :aria-label="t('服务地址')" :options="deepLEndpoints.map((value, index) => ({ value, label: index === 0 ? 'DeepL API Free' : 'DeepL API Pro' }))" @update:model-value="autosave.commit" /></label>
        <label v-else>{{ t("完整接口地址") }}<input v-model.trim="draft.endpoint" type="url" :placeholder='t("https://服务域名/v1/chat/completions")' autocomplete="off" /></label>
        <label v-if="draft.provider === 'online-llm'">{{ t("模型名称") }}<input v-model.trim="draft.model" :placeholder='t("填写服务支持的模型 ID")' /></label>
        <label>API Key<input v-model="secret" type="password" autocomplete="new-password" :placeholder='t("留空保留现有密钥")' /></label>
        <p :class="{ 'status-ok': configured }" role="status"><Check v-if="configured" :size="14" />{{ configured === null ? t("正在检查…") : configured ? t("已配置") : t("未配置") }}<InfoTip :text="t('密钥随当前服务地址保存。')" /></p>
        </template>
      </div>
      <div class="translation-options">
        <label>{{ t("默认目标语言") }}<SelectControl v-model="draft.targetLanguage" :disabled="busy || !ready" :aria-label="t('目标语言')" :options="languages.map(item => ({ value: item.value, label: t(item.label) }))" @update:model-value="autosave.commit" /></label>
        <label>{{ t("超时（秒）") }}<input v-model.number="draft.timeoutSeconds" type="number" min="5" max="120" /></label>
        <SettingRow :label='t("翻译后自动复制译文")' for="translation-auto-copy"><SwitchControl id="translation-auto-copy" v-model="draft.autoCopy" @change="autosave.commit" /></SettingRow>
        <div v-if="local" class="idle-field"><div class="field-label"><label for="translation-idle-seconds">{{ t("闲置回收（秒）") }}</label><InfoTip :text="t('0 不回收；可设为 30～3600 秒。')" /></div><input id="translation-idle-seconds" v-model.number="draft.localIdleSeconds" type="number" min="0" max="3600" step="1" /></div>
      </div>
      <div class="actions"><span v-if="autosave.saving.value || autosave.saved.value" class="settings-save-status" :class="{ 'status-ok': !autosave.saving.value }" role="status"><Check v-if="!autosave.saving.value" :size="14" />{{ autosave.saving.value ? t("正在保存…") : t("已自动保存") }}</span><Button variant="outline" :disabled="!local && !localEndpoint && !configured" @click="credentialAction('test')">{{ t("测试已保存配置") }}</Button><InfoTip v-if="!local && !localEndpoint" :text="t('测试会发送 Hello，可能产生费用。')" /><Button v-if="!local && !draft.serviceId" variant="ghost" :disabled="!configured" @click="credentialAction('delete')">{{ t("删除已保存密钥") }}</Button></div>
    </fieldset>
    <p v-if="autosave.error.value" class="notice error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t("重试") }}</Button></p>
    <p v-if="message" :class="{ 'status-ok': !busy }" role="status"><Check v-if="!busy" :size="14" />{{ t(message) }}</p><p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    </section>
  </section>
</template>

<style scoped>
.ai-settings { width:100%; max-width:920px; margin-inline:auto; font-size:13px; }
.recognition-section { padding:0; }
.recognition-section:last-child { border-bottom:0; }
h3 { font-size:14px; font-weight:600; margin:0 0 12px; }
p { color:#57606a; margin:8px 0; }
fieldset { border:0; padding:0; min-width:0; }
label { display:grid; grid-template-columns:124px minmax(0,1fr); align-items:center; gap:12px; margin:8px 0; }
input, select { height:32px; border:1px solid #d8dee4; border-radius:4px; padding:4px 8px; min-width:0; width:100%; color:inherit; }
input:focus-visible, select:focus-visible { outline:2px solid #2563eb; outline-offset:2px; }
.provider-field { grid-template-columns:110px minmax(0,240px); margin-top:0; }
.local-resources { margin:0 0 16px; }
.model-row { display:grid; grid-template-columns:minmax(120px,1fr) minmax(100px,1fr) minmax(0,auto); align-items:center; gap:8px 16px; padding:8px 0; min-height:48px; border-bottom:1px solid #edf0f2; }
.model-row:last-child { border-bottom:0; }
.runtime-status { grid-column:span 2; }
.model-name { overflow-wrap:anywhere; }
.model-actions { display:flex; align-items:center; justify-content:flex-end; flex-wrap:wrap; gap:6px; }
.translation-options { display:flex; flex-direction:column; gap:8px; margin:16px 0; }
.translation-options label { display:grid; grid-template-columns:124px minmax(0,1fr); align-items:center; gap:12px; margin:0; font-size:13px; }
.translation-options :is(input,select) { font-size:13px; color:#24292f; }
.idle-field { display:grid; grid-template-columns:124px minmax(0,1fr); gap:12px; align-items:center; }
.field-label { display:flex; align-items:center; gap:2px; }
.translation-options .field-label label { display:block; }
.online-fields { width:100%; }
.credential-status { display:flex; align-items:center; gap:4px; font-size:12px; margin:8px 0; }
.connection-summary { margin:0; }
.connection-summary > div { display:grid; grid-template-columns:124px minmax(0,1fr); gap:12px; align-items:start; padding:8px 0; }
.connection-summary dt { color:var(--desk-muted,#57606a); }
.connection-summary dd { margin:0; overflow-wrap:anywhere; }
.actions { display:flex; align-items:center; flex-wrap:wrap; gap:8px; padding-top:12px; border-top:1px solid var(--desk-border,#e1e4e8); }
.hint { font-size:12px; line-height:1.6; }
.error { color:#b91c1c; }
.ai-settings :deep(.setting-row) { min-height:48px; padding-top:0; }
[data-theme=dark] .recognition-section, [data-theme=dark] .model-row { border-color:#39424e; }
[data-theme=dark] :is(p,.translation-options label) { color:#a6b2c0; }
[data-theme=dark] .translation-options :is(input,select) { color:#e6edf3; }
[data-theme=dark] .error { color:#ffb8bd; }
@media (max-width:760px) {
  .model-row { grid-template-columns:minmax(0,1fr) auto; }
  .runtime-status { grid-column:span 2; }
  .model-actions { grid-column:span 2; justify-content:flex-start; }
}
@media (max-width:540px) {
  .provider-field, .online-fields label, .translation-options label, .idle-field, .connection-summary > div { grid-template-columns:1fr; gap:6px; }
}
</style>
