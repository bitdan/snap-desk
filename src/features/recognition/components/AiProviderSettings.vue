<script setup lang="ts">
import ProviderMenu, { type ProviderMenuItem } from './ProviderMenu.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { updateSettings } from '@/features/settings/settings-write'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import { defaultAiSettings, targetLanguages, translationPresets, validateAiSettings } from '@/features/recognition/ai'
import type { AiSettings, LocalTranslationStatus } from '@/features/recognition/ai'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import InfoTip from '@/shared/ui/InfoTip.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'
import { cloudFirst, connectionMode, supportsTranslation, translationPresetId, translationService, type RecognitionService } from '@/features/recognition/services'

/** 翻译页只选择引擎和翻译偏好；连接、密钥、本地资源与测试都在服务管理中维护。 */
const emit = defineEmits<{ manage: [] }>()
const activeSettings = ref<AiSettings | null>(null)
// 未能迁移为服务的旧版直接配置只保留一个可切回的入口，不再提供编辑。
const legacy = ref<AiSettings | null>(null)
const services = ref<RecognitionService[]>([])
const draft = ref(defaultAiSettings()), configured = ref<boolean | null>(null)
const localStatus = ref<LocalTranslationStatus | null>(null)
let disposed = false, credentialStatusRevision = 0
const localEndpoint = computed(() => { try { return ['localhost', '127.0.0.1', '[::1]'].includes(new URL(draft.value.endpoint).hostname) } catch { return false } })
const local = computed(() => draft.value.provider === 'local-ctranslate2')
const languages = computed(() => local.value ? targetLanguages.slice(0, 2) : targetLanguages)
const localReady = computed(() => !!localStatus.value?.runtimeReady && !!localStatus.value.models?.some(pack => pack.ready))
watch(local, value => { if (value && !['zh-CN', 'en'].includes(draft.value.targetLanguage)) draft.value.targetLanguage = 'zh-CN' })
watch(() => [draft.value.endpoint, draft.value.provider, draft.value.serviceId], () => { credentialStatusRevision++; configured.value = null }, { flush: 'sync' })
const ready = ref(false), error = ref('')
async function refreshCredential() {
  const revision = credentialStatusRevision
  if (local.value) return
  const value = await api.aiProviderStatus().catch(() => null)
  if (!disposed && revision === credentialStatusRevision) configured.value = value
}
async function loadSettings() {
  try {
    const settings = await api.status()
    if (disposed) return
    services.value = settings.settings.recognitionServices
    const saved = settings.settings.ai
    const service = services.value.find(item => item.id === saved.serviceId)
    draft.value = service && saved.provider !== 'local-ctranslate2' ? translationService(saved, service) : { ...saved }; ready.value = true
    activeSettings.value = { ...draft.value }
    if (!draft.value.serviceId && draft.value.provider !== 'local-ctranslate2') legacy.value = { ...draft.value }
    await refreshCredential()
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
onMounted(() => {
  // 配置/凭证与引擎校验独立返回，慢校验或失败不能拖住整个表单。
  void loadSettings()
  void api.translationModelStatus().then(value => { if (!disposed) localStatus.value = value }).catch(() => { /* 状态未读取时只少一条提示。 */ })
})
onUnmounted(() => { disposed = true })
/** 只覆盖 AI 字段，读取最新配置以保留其他页面已保存的设置。 */
const autosave = useAutoSave(async () => {
  if (!ready.value) return
  const ai = { ...draft.value }
  const saved = await updateSettings(current => ({ ...current, ai }))
  if (!disposed) activeSettings.value = { ...saved.ai }
  await refreshCredential()
}, () => validateAiSettings(draft.value))
function selectService(id: string) {
  const service = services.value.find(item => item.id === id)
  if (service) draft.value = translationService(draft.value, service)
  else draft.value.serviceId = ''
  autosave.commit()
}
/** 下拉只做切换：本地引擎、已保存的服务，以及未能迁移的旧版直接配置。 */
const menuValue = (value: AiSettings) => value.provider === 'local-ctranslate2' ? 'offline' : value.serviceId ? `service:${value.serviceId}` : 'legacy'
const selectedMenu = computed(() => menuValue(draft.value))
const savedOptions = computed(() => services.value.filter(supportsTranslation).map(item => ({ value: `service:${item.id}`, label: item.name, description: item.model || item.protocol, connection: connectionMode(item.endpoint), presetId: translationPresetId(item), endpoint: item.endpoint })))
const activeMenu = computed(() => activeSettings.value ? menuValue(activeSettings.value) : '')
const legacyLabel = computed(() => {
  const value = legacy.value
  if (!value) return ''
  const name = value.provider === 'deepl' ? 'DeepL' : t(translationPresets.find(item => item.endpoint === value.endpoint)?.label ?? '自定义')
  return `${name} · ${t('旧配置')}`
})
const menuOptions = computed(() => cloudFirst<ProviderMenuItem>([
  { value: 'offline', label: 'CTranslate2', connection: 'offline' as const },
  ...(legacy.value ? [{ value: 'legacy', label: legacyLabel.value, description: legacy.value.model, connection: connectionMode(legacy.value.endpoint), presetId: legacy.value.provider === 'deepl' ? 'deepl' : translationPresets.find(item => item.endpoint === legacy.value!.endpoint)?.value, endpoint: legacy.value.endpoint }] : []),
  ...savedOptions.value,
]).map(item => ({ ...item, active: item.value === activeMenu.value })))
const activeLabel = computed(() => {
  const item = menuOptions.value.find(item => item.value === activeMenu.value)
  return [item?.label, activeSettings.value?.provider !== 'local-ctranslate2' ? activeSettings.value?.model : ''].filter(Boolean).join(' · ')
})
function selectMenu(value: string) {
  if (!ready.value) return
  if (value === 'offline') { draft.value = { ...draft.value, provider: 'local-ctranslate2', serviceId: '' }; autosave.commit(); return }
  if (value === 'legacy' && legacy.value) { draft.value = { ...draft.value, ...legacy.value, serviceId: '', targetLanguage: draft.value.targetLanguage, timeoutSeconds: draft.value.timeoutSeconds, autoCopy: draft.value.autoCopy, overlay: draft.value.overlay }; autosave.commit(); return }
  if (value.startsWith('service:')) selectService(value.slice(8))
}
</script>

<template>
  <section class="ai-settings">
    <!-- 生效变化由读屏播报；视觉状态由菜单勾选统一表达。 -->
    <p class="sr-only" role="status" data-current-translation>{{ activeSettings ? t('当前翻译：{0}', { 0: activeLabel }) : t('正在读取配置…') }}</p>
    <fieldset :disabled="!ready" @change="autosave.commit" @focusout="autosave.flush">

      <h2 class="section-heading">{{ t('引擎') }}</h2>
      <div class="settings-card recognition-card">
      <div class="setting-row"><span>{{ t('翻译引擎') }}<small>{{ local ? t('离线翻译，文字不上传') : localEndpoint ? t('调用本机兼容接口') : t('在线服务，文字会上传') }}</small></span><ProviderMenu :model-value="selectedMenu" :options="menuOptions" :label="t('翻译引擎')" :disabled="!ready" :manage-label="t('服务管理…')" @update:model-value="selectMenu" @manage="emit('manage')" /></div>
      <div class="recognition-detail">
        <p v-if="local && localStatus" class="engine-status" :class="{ 'status-ok': localReady }" role="status"><Check v-if="localReady" :size="14" />{{ localReady ? t('本地引擎已安装') : t('本地引擎或语言包未安装，请在服务管理中安装。') }}</p>
        <p v-else-if="!local" class="engine-status" :class="{ 'status-ok': configured }" role="status"><Check v-if="configured" :size="14" />{{ configured === null ? t('凭据状态未读取') : configured ? t('已配置密钥') : localEndpoint ? t('本机服务可不填密钥') : t('未配置密钥，请在服务管理中填写。') }}</p>
      </div>
      </div>
      <h2 class="section-heading">{{ t('翻译设置') }}</h2>
      <div class="settings-card"><div class="translation-options">
        <label>{{ t("默认目标语言") }}<SelectControl v-model="draft.targetLanguage" :disabled="!ready" :aria-label="t('目标语言')" :options="languages.map(item => ({ value: item.value, label: t(item.label) }))" @update:model-value="autosave.commit" /></label>
        <label>{{ t("超时（秒）") }}<input v-model.number="draft.timeoutSeconds" type="number" min="5" max="120" /></label>
        <SettingRow :label='t("翻译后自动复制译文")' for="translation-auto-copy"><SwitchControl id="translation-auto-copy" v-model="draft.autoCopy" @change="autosave.commit" /></SettingRow>
        <SettingRow :label='t("截图翻译覆盖原文")' for="translation-overlay"><SwitchControl id="translation-overlay" v-model="draft.overlay" @change="autosave.commit" /></SettingRow>
        <div v-if="local" class="idle-field"><div class="field-label"><label for="translation-idle-seconds">{{ t("闲置回收（秒）") }}</label><InfoTip :text="t('0 不回收；可设为 30～3600 秒。')" /></div><input id="translation-idle-seconds" v-model.number="draft.localIdleSeconds" type="number" min="0" max="3600" step="1" /></div>
      </div></div>
      <div class="actions"><span v-if="autosave.saving.value || autosave.saved.value" class="settings-save-status" :class="{ 'status-ok': !autosave.saving.value }" role="status"><Check v-if="!autosave.saving.value" :size="14" />{{ autosave.saving.value ? t("正在保存…") : t("已自动保存") }}</span></div>
    </fieldset>
    <p v-if="autosave.error.value" class="notice error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t("重试") }}</Button></p>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
  </section>
</template>

<style scoped>
.ai-settings { width:100%; max-width:920px; margin-inline:auto; font-size:13px; }
.recognition-section { padding:0; }
.recognition-section:last-child { border-bottom:0; }
h3 { font-size:14px; font-weight:600; margin:0 0 12px; }
p { color:var(--desk-muted,#5f6670); margin:8px 0; }
fieldset { border:0; padding:0; min-width:0; }
label { display:grid; grid-template-columns:112px minmax(0,1fr); align-items:center; gap:12px; margin:4px 0; }
input, select { height:28px; border:1px solid var(--desk-line,#d3d8de); border-radius:6px; background:var(--desk-surface,#fff); padding:4px 8px; min-width:0; width:100%; color:inherit; }
input:focus-visible, select:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:2px; }
.provider-field { grid-template-columns:110px minmax(0,240px); margin-top:0; }
.local-resources { margin:0; }
.model-row { display:grid; grid-template-columns:minmax(120px,1fr) minmax(100px,1fr) minmax(0,auto); align-items:center; gap:8px 16px; padding:8px 0; min-height:48px; border-bottom:1px solid var(--desk-sep,#eceef1); }
.model-row:last-child { border-bottom:0; }
.runtime-status { grid-column:span 2; }
.model-name { overflow-wrap:anywhere; }
.model-actions { display:flex; align-items:center; justify-content:flex-end; flex-wrap:wrap; gap:6px; }
.translation-options { display:flex; flex-direction:column; gap:4px; margin:0; padding:8px 12px 8px 0; }
.translation-options label { display:grid; grid-template-columns:112px minmax(0,1fr); align-items:center; gap:12px; margin:0; font-size:13px; }
.translation-options :is(input,select) { font-size:13px; color:var(--desk-text,#1f2328); }
.idle-field { display:grid; grid-template-columns:124px minmax(0,1fr); gap:12px; align-items:center; }
.field-label { display:flex; align-items:center; gap:2px; }
.translation-options .field-label label { display:block; }
.engine-status { display:flex; align-items:center; gap:4px; margin:4px 0; font-size:12px; }
.credential-status { display:flex; align-items:center; gap:4px; font-size:12px; margin:8px 0; }
.connection-summary { margin:0; }
.connection-summary > div { display:grid; grid-template-columns:124px minmax(0,1fr); gap:12px; align-items:start; padding:8px 0; }
.connection-summary dt { color:var(--desk-muted,#57606a); }
.connection-summary dd { margin:0; overflow-wrap:anywhere; }
.actions { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin-top:10px; }
.hint { margin:6px 4px 0; color:var(--desk-muted,#5f6670); font-size:12px; line-height:1.6; }
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
