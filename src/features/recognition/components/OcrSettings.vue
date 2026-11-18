<script setup lang="ts">
import { Check } from 'lucide-vue-next'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/services/api'
import { cloudFirst, connectionMode, serviceKind, servicePresetId, supportsOcr, type RecognitionService } from '@/features/recognition/services'
import { defaultOcrSettings, type OcrSettings, type SystemOcrStatus, type WechatOcrStatus } from '@/features/recognition/ocr'
import { updateSettings } from '@/features/settings/settings-write'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { t, te } from '@/shared/i18n/i18n'
import { macPlatform } from '@/shared/platform'
import ProviderMenu from './ProviderMenu.vue'
import type { ProviderMenuItem } from './ProviderMenu.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'

const emit = defineEmits<{ manage: [] }>()
const services = ref<RecognitionService[]>([])
const draft = ref(defaultOcrSettings()), ready = ref(false), error = ref('')
const activeSettings = ref<OcrSettings | null>(null)
const activeMenu = computed(() => !activeSettings.value ? '' : activeSettings.value.provider === 'service' ? `service:${activeSettings.value.serviceId}` : activeSettings.value.provider)
const selectedMenu = computed(() => draft.value.provider === 'service' ? `service:${draft.value.serviceId}` : draft.value.provider)
const wechat = ref<WechatOcrStatus | null>(null), checkingWechat = ref(false), wechatError = ref('')
const system = ref<SystemOcrStatus | null>(null), checking = ref(false)
const resources = ref<Awaited<ReturnType<typeof api.resourceStatus>> | null>(null)
// 在线服务统一在服务管理中添加和编辑；此处只列出已保存且支持图片的连接，选择即切换。
const engineOptions = computed(() => cloudFirst<ProviderMenuItem>([
  ...services.value.filter(supportsOcr).map(item => ({ value: `service:${item.id}`, label: item.name, description: item.model || t(serviceKind(item)), connection: connectionMode(item.endpoint), presetId: servicePresetId(item), endpoint: item.endpoint })),
  { value: 'local-ppocr', label: 'PaddleOCR', description: 'PP-OCRv6', connection: 'offline' as const },
  { value: 'windows-media', label: macPlatform ? 'Apple Vision' : 'Windows OCR', connection: 'offline' as const, disabled: !system.value?.supported },
  // 检测未完成或失败时仍允许打开面板查看原因和重试，仅明确不支持的平台禁用。
  { value: 'wechat-ocr', label: t('WeChat OCR（实验性）'), connection: 'offline' as const, disabled: wechat.value?.supported === false },
]).map(item => ({ ...item, active: item.value === activeMenu.value })))
const activeLabel = computed(() => {
  const item = engineOptions.value.find(item => item.value === activeMenu.value)
  return item ? [item.label, 'description' in item ? item.description : ''].filter(Boolean).join(' · ') : ''
})
const currentService = computed(() => draft.value.provider === 'service' ? services.value.find(item => item.id === draft.value.serviceId) : undefined)
const engineHint = computed(() => {
  const item = engineOptions.value.find(option => option.value === selectedMenu.value)
  return !item ? '' : item.connection === 'online' ? t('在线服务，图片会上传') : t('离线识别，图片不上传')
})
const credential = ref<boolean | null>(null)
let credentialRevision = 0
async function refreshCredential() {
  const revision = ++credentialRevision, service = currentService.value
  credential.value = null
  if (!service) return
  const value = await api.recognitionServiceStatus(service.id).catch(() => null)
  if (!disposed && revision === credentialRevision) credential.value = value
}
function selectEngine(value: string) {
  if (!ready.value) return
  const service = value.startsWith('service:')
  draft.value.provider = service ? 'service' : value as OcrSettings['provider']
  draft.value.serviceId = service ? value.slice(8) : ''
  void refreshCredential()
  autosave.commit()
}
const windows = computed(() => draft.value.provider === 'windows-media')
let disposed = false
const languageOptions = computed(() => {
  const options = [{ value: 'auto', label: t('跟随系统首选语言') }, ...(system.value?.languages ?? []).map(item => ({ value: item.tag, label: `${item.name} (${item.tag})` }))]
  if (!options.some(item => item.value === draft.value.language)) options.push({ value: draft.value.language, label: draft.value.language })
  return options
})
const autosave = useAutoSave(async () => {
  const ocr = { ...draft.value }
  const saved = await updateSettings(current => ({ ...current, ocr }))
  if (!disposed) activeSettings.value = { ...saved.ocr }
})
async function refreshSystem() {
  if (checking.value) return
  checking.value = true; error.value = ''
  try {
    const value = await api.systemOcrStatus()
    if (!disposed) system.value = value
  } catch (e) { if (!disposed) { system.value = null; error.value = String(e) } }
  finally { if (!disposed) checking.value = false }
}
async function refreshWechat() {
  if (checkingWechat.value) return
  checkingWechat.value = true; wechatError.value = ''
  try {
    const value = await api.wechatOcrStatus()
    if (!disposed) wechat.value = value
  } catch (e) { if (!disposed) { wechat.value = null; wechatError.value = String(e) } }
  finally { if (!disposed) checkingWechat.value = false }
}
onMounted(() => {
  void api.status().then(value => { if (!disposed) { draft.value = { ...value.settings.ocr }; activeSettings.value = { ...value.settings.ocr }; services.value = value.settings.recognitionServices; ready.value = true; void refreshCredential() } }).catch(e => { if (!disposed) error.value = String(e) })
  void refreshSystem()
  void refreshWechat()
  void api.resourceStatus().then(value => { if (!disposed) resources.value = value }).catch(() => { /* 状态未读取时只少一条提示。 */ })
})
onUnmounted(() => { disposed = true })
</script>

<template>
  <div class="ocr-settings">
    <h2 class="section-heading">{{ t('引擎') }}</h2>
    <div class="settings-card recognition-card">
    <div class="setting-row"><span>{{ t('OCR 引擎') }}<small v-if="engineHint">{{ engineHint }}</small></span><ProviderMenu :model-value="selectedMenu" :label="t('OCR 引擎')" :disabled="!ready || autosave.saving.value" :options="engineOptions" :manage-label="t('服务管理…')" @update:model-value="selectEngine" @manage="emit('manage')" /></div>
    <div class="recognition-detail">
    <!-- 保留生效状态播报，避免与左侧菜单的当前使用标记重复显示。 -->
    <p class="sr-only" role="status" data-current-ocr>{{ activeSettings ? t('当前 OCR：{0}', { 0: activeLabel }) : t('正在读取配置…') }}</p>
    <p v-if="currentService" class="engine-status" :class="{ 'status-ok': credential }" role="status"><Check v-if="credential" :size="14" />{{ credential === null ? t('凭据状态未读取') : credential ? t('已配置密钥') : t('未配置密钥，请在服务管理中填写。') }}</p>
    <template v-else-if="windows">
      <label class="ocr-field">{{ t('识别语言') }}<SelectControl v-model="draft.language" :aria-label="t('识别语言')" :options="languageOptions" :disabled="!ready || checking || autosave.saving.value || !system?.supported || !!system?.error" @update:model-value="autosave.commit" /></label>
      <p class="engine-status" role="status">{{ checking ? t('正在检查系统语言包…') : !system ? t('状态未读取') : !system.supported ? t('Windows 系统 OCR 仅支持 Windows。') : system.error ? te(system.error) : macPlatform ? t('支持 {0} 种识别语言', { 0: system.languages.length }) : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装 Windows OCR 语言包，请在 Windows 语言设置中安装。') }}</p>
    </template>
    <p v-else-if="draft.provider === 'wechat-ocr'" class="engine-status" role="status">{{ checkingWechat ? t('正在检测 WeChat OCR…') : wechatError ? te(wechatError) : !wechat ? t('状态未读取') : wechat.error ? te(wechat.error) : wechat.detected ? t('已找到 WeChat OCR 组件') : t('未找到 WeChat OCR 组件') }}</p>
    <p v-else-if="draft.provider === 'local-ppocr' && resources" class="engine-status" :class="{ 'status-ok': resources.ocrReady }" role="status"><Check v-if="resources.ocrReady" :size="14" />{{ resources.ocrReady ? t('已就绪') : t('未下载识别资源，请在服务管理中下载。') }}</p>
    </div>
    </div>
    <h2 v-if="ready" class="section-heading">{{ t('识别后') }}</h2>
    <section v-if="ready" class="settings-card" :aria-label="t('识别后')">
      <SettingRow :label="t('识别后自动复制文字')" for="ocr-auto-copy"><SwitchControl id="ocr-auto-copy" v-model="draft.autoCopy" @change="autosave.commit" /></SettingRow>
      <SettingRow :label="t('识别后自动合并换行')" for="ocr-merge-lines"><SwitchControl id="ocr-merge-lines" v-model="draft.mergeLines" @change="autosave.commit" /></SettingRow>
    </section>
    <div v-if="autosave.saving.value || autosave.saved.value" class="save-status" :class="{ 'status-ok': !autosave.saving.value }" role="status"><Check v-if="!autosave.saving.value" :size="14" />{{ autosave.saving.value ? t('正在保存…') : t('已自动保存') }}</div>
    <p v-if="autosave.error.value" class="error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t('重试') }}</Button></p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}</p>
  </div>
</template>

<style scoped>
.ocr-settings { font-size:13px; }
.ocr-field { display:grid; grid-template-columns:112px minmax(0,1fr); align-items:center; gap:12px; margin:4px 0; }
.system-status { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin:8px 0; }
.system-status span { overflow-wrap:anywhere; }
.hint, .save-status { color:var(--desk-muted, #57606a); font-size:12px; line-height:1.6; margin:6px 4px 0; }
.engine-status { display:flex; align-items:center; gap:4px; margin:4px 0; font-size:12px; color:var(--desk-muted,#57606a); overflow-wrap:anywhere; }
.engine-status.status-ok { color:var(--ui-success-fg,#24834b); }
.save-status.status-ok { color:var(--ui-success-fg,#24834b); }
.error { color:#b91c1c; overflow-wrap:anywhere; }
</style>
