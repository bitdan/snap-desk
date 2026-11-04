<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/services/api'
import { cloudFirst, connectionMode, serviceKind, supportsOcr, type RecognitionService } from '@/features/recognition/services'
import { defaultOcrSettings, type OcrSettings, type SystemOcrStatus, type WechatOcrStatus } from '@/features/recognition/ocr'
import { updateSettings } from '@/features/settings/settings-write'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { t, te } from '@/shared/i18n/i18n'
import ServiceSettings from './ServiceSettings.vue'
import ProviderMenu from './ProviderMenu.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import ResourceSettings from '@/features/resources/components/ResourceSettings.vue'

const editingServiceId = ref('')
const configuringPreset = ref(''), configuringBusy = ref(false)
const services = ref<RecognitionService[]>([])
const draft = ref(defaultOcrSettings()), ready = ref(false), error = ref('')
const activeSettings = ref<OcrSettings | null>(null)
const activeMenu = computed(() => !activeSettings.value ? '' : activeSettings.value.provider === 'service' ? `service:${activeSettings.value.serviceId}` : activeSettings.value.provider)
const wechat = ref<WechatOcrStatus | null>(null), checkingWechat = ref(false), wechatError = ref('')
const system = ref<SystemOcrStatus | null>(null), checking = ref(false)
// 有已保存连接时，用连接替代同供应商的添加入口，避免出现两个同名选项。
const engineOptions = computed(() => {
  const available = services.value.filter(supportsOcr)
  const option = (item: RecognitionService) => ({ value: `service:${item.id}`, label: item.name, description: item.model || t(serviceKind(item)), connection: connectionMode(item.endpoint) })
  const cloud = [{ protocol: 'ocr-space', label: 'OCR.space' }, { protocol: 'paddle-ocr', label: 'Paddle API' }].flatMap(preset => {
    const saved = available.filter(item => item.protocol === preset.protocol)
    return saved.length ? saved.map(option) : [{ value: `preset:${preset.protocol}`, label: preset.label, description: '', connection: 'online' as const }]
  })
  return cloudFirst([
    ...cloud,
    ...available.filter(item => !['ocr-space', 'paddle-ocr'].includes(item.protocol)).map(option),
    { value: 'local-ppocr', label: 'PaddleOCR', description: 'PP-OCRv6', connection: 'offline' as const },
    { value: 'windows-media', label: 'Windows OCR', connection: 'offline' as const, disabled: !system.value?.supported },
    // 检测未完成或失败时仍允许打开面板查看原因和重试，仅明确不支持的平台禁用。
    { value: 'wechat-ocr', label: t('WeChat OCR（实验性）'), connection: 'offline' as const, disabled: wechat.value?.supported === false },
  ]).map(item => ({ ...item, active: item.value === activeMenu.value }))
})
const activeLabel = computed(() => {
  const item = engineOptions.value.find(item => item.value === activeMenu.value)
  return item ? [item.label, 'description' in item ? item.description : ''].filter(Boolean).join(' · ') : ''
})
function selectEngine(value: string) {
  if (configuringBusy.value || !ready.value) return
  editingServiceId.value = ''
  if (value.startsWith('preset:')) { configuringPreset.value = value.slice(7); return }
  configuringPreset.value = ''
  if (value.startsWith('service:')) editingServiceId.value = value.slice(8)
  draft.value.provider = value.startsWith('service:') ? 'service' : value as OcrSettings['provider']
  draft.value.serviceId = value.startsWith('service:') ? value.slice(8) : ''
  autosave.commit()
}
/** 表单保存服务与凭据成功后，再独立保存 OCR 绑定；失败沿用自动保存的重试入口。 */
function serviceSaved(service: RecognitionService) {
  configuringBusy.value = false
  editingServiceId.value = ''
  services.value = services.value.some(item => item.id === service.id) ? services.value.map(item => item.id === service.id ? service : item) : [...services.value, service]
  configuringPreset.value = ''
  draft.value = { ...draft.value, provider: 'service', serviceId: service.id }
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
  void api.status().then(value => { if (!disposed) { draft.value = { ...value.settings.ocr }; activeSettings.value = { ...value.settings.ocr }; services.value = value.settings.recognitionServices; ready.value = true } }).catch(e => { if (!disposed) error.value = String(e) })
  void refreshSystem()
  void refreshWechat()
})
onUnmounted(() => { disposed = true })
</script>

<template>
  <div class="ocr-settings recognition-settings-layout">
    <aside class="recognition-settings-sidebar"><ProviderMenu :model-value="configuringPreset ? `preset:${configuringPreset}` : draft.provider === 'service' ? `service:${draft.serviceId}` : draft.provider" :label="t('OCR 引擎')" :disabled="!ready || autosave.saving.value || configuringBusy" :options="engineOptions" @update:model-value="selectEngine" /></aside>
    <div class="recognition-settings-content">
    <p class="recognition-current-provider" role="status" data-current-ocr>{{ activeSettings ? t('当前 OCR：{0}', { 0: activeLabel }) : t('正在读取配置…') }}</p>
    <ServiceSettings v-if="configuringPreset || editingServiceId" :key="editingServiceId || configuringPreset" embedded :preset-id="configuringPreset || undefined" :service-id="editingServiceId || undefined" @saved="serviceSaved" @busy-change="configuringBusy = $event" />
    <template v-else-if="windows">
      <label class="ocr-field">{{ t('识别语言') }}<SelectControl v-model="draft.language" :aria-label="t('识别语言')" :options="languageOptions" :disabled="!ready || checking || autosave.saving.value || !system?.supported || !!system?.error" @update:model-value="autosave.commit" /></label>
      <div class="system-status"><span role="status">{{ checking ? t('正在检查系统语言包…') : !system ? t('状态未读取') : !system.supported ? t('Windows 系统 OCR 仅支持 Windows。') : system.error ? te(system.error) : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装 Windows OCR 语言包，请在 Windows 语言设置中安装。') }}</span><Button variant="outline" :disabled="checking" @click="refreshSystem">{{ t('刷新语言包') }}</Button></div>
      <p class="hint">{{ t('使用系统离线识别，图片不上传；失败时不切换其他引擎。') }}</p>
    </template>
    <div v-else-if="draft.provider === 'wechat-ocr'" class="system-status">
      <span role="status">{{ checkingWechat ? t('正在检测 WeChat OCR…') : wechatError ? te(wechatError) : !wechat ? t('状态未读取') : wechat.error ? te(wechat.error) : wechat.detected ? t('已找到 WeChat OCR 组件') : t('未找到 WeChat OCR 组件') }}</span>
      <Button variant="outline" :disabled="checkingWechat" @click="refreshWechat">{{ t('重新检测') }}</Button>
    </div>
    <ResourceSettings v-else-if="ready && draft.provider === 'local-ppocr'" kind="ocr" />
    <p v-else-if="draft.provider === 'service'" class="hint">{{ t('OCR 会将选区图片发送到此服务，可能产生费用。') }}</p>
    <p v-if="ready && !configuringPreset && !editingServiceId" class="hint">{{ t('更多在线 OCR 或本机模型可在服务管理中添加。') }}</p>
    <div v-if="!configuringPreset && !editingServiceId" class="save-status" role="status">{{ autosave.saving.value ? t('正在保存…') : autosave.saved.value ? t('已自动保存') : t('更改自动保存') }}</div>
    <p v-if="autosave.error.value" class="error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t('重试') }}</Button></p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}</p>
    </div>
  </div>
</template>

<style scoped>
.ocr-settings { font-size:13px; }
.ocr-field { display:grid; grid-template-columns:92px minmax(0,1fr); align-items:center; gap:12px; margin:8px 0; }
.system-status { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin:8px 0; }
.system-status span { overflow-wrap:anywhere; }
.hint, .save-status { color:var(--desk-muted, #57606a); font-size:12px; line-height:1.6; margin:8px 0; }
.error { color:#b91c1c; overflow-wrap:anywhere; }
</style>
