<script setup lang="ts">
import { Check } from 'lucide-vue-next'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/services/api'
import { cloudFirst, connectionMode, serviceKind, servicePresetId, supportsOcr, type RecognitionService } from '@/features/recognition/services'
import { defaultOcrSettings, type OcrSettings, type SystemOcrStatus, type WechatOcrStatus } from '@/features/recognition/ocr'
import { updateSettings } from '@/features/settings/settings-write'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { t, te } from '@/shared/i18n/i18n'
import ServiceSettings from './ServiceSettings.vue'
import ProviderMenu from './ProviderMenu.vue'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import InfoTip from '@/shared/ui/InfoTip.vue'
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
  const presets = [{ id: 'ocr-space', label: 'OCR.space' }, { id: 'paddle-ocr', label: 'Paddle API' }, { id: 'deepseek', label: 'DeepSeek' }]
  const cloud = presets.flatMap(preset => {
    const saved = available.filter(item => servicePresetId(item) === preset.id)
    return saved.length ? saved.map(option) : [{ value: `preset:${preset.id}`, label: preset.label, description: '', connection: 'online' as const }]
  })
  return cloudFirst([
    ...cloud,
    ...available.filter(item => !presets.some(preset => preset.id === servicePresetId(item))).map(option),
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
    <!-- 保留生效状态播报，避免与左侧菜单的当前使用标记重复显示。 -->
    <p class="sr-only" role="status" data-current-ocr>{{ activeSettings ? t('当前 OCR：{0}', { 0: activeLabel }) : t('正在读取配置…') }}</p>
    <ServiceSettings v-if="configuringPreset || editingServiceId" :key="editingServiceId || configuringPreset" embedded :preset-id="configuringPreset || undefined" :service-id="editingServiceId || undefined" @saved="serviceSaved" @busy-change="configuringBusy = $event" />
    <template v-else-if="windows">
      <label class="ocr-field">{{ t('识别语言') }}<SelectControl v-model="draft.language" :aria-label="t('识别语言')" :options="languageOptions" :disabled="!ready || checking || autosave.saving.value || !system?.supported || !!system?.error" @update:model-value="autosave.commit" /></label>
      <div class="system-status"><span role="status">{{ checking ? t('正在检查系统语言包…') : !system ? t('状态未读取') : !system.supported ? t('Windows 系统 OCR 仅支持 Windows。') : system.error ? te(system.error) : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装 Windows OCR 语言包，请在 Windows 语言设置中安装。') }}</span><Button variant="outline" :disabled="checking" @click="refreshSystem">{{ t('刷新语言包') }}</Button></div>
    </template>
    <div v-else-if="draft.provider === 'wechat-ocr'" class="system-status">
      <span role="status">{{ checkingWechat ? t('正在检测 WeChat OCR…') : wechatError ? te(wechatError) : !wechat ? t('状态未读取') : wechat.error ? te(wechat.error) : wechat.detected ? t('已找到 WeChat OCR 组件') : t('未找到 WeChat OCR 组件') }}</span>
      <Button variant="outline" :disabled="checkingWechat" @click="refreshWechat">{{ t('重新检测') }}</Button>
    </div>
    <ResourceSettings v-else-if="ready && draft.provider === 'local-ppocr'" kind="ocr" />
    <!-- 识别后的动作与引擎无关；配置在线服务时让位给服务表单。 -->
    <section v-if="ready && !configuringPreset && !editingServiceId" class="ocr-behavior" :aria-label="t('识别后')">
      <div class="setting-row"><label for="ocr-auto-copy">{{ t('识别后自动复制文字') }}</label><input id="ocr-auto-copy" v-model="draft.autoCopy" type="checkbox" role="switch" @change="autosave.commit" /></div>
      <div class="setting-row"><div class="flex items-center gap-1"><label for="ocr-merge-lines">{{ t('识别后自动合并换行') }}</label><InfoTip :text="t('把同一段落的多行拼成一行，空行仍分段；可在结果里还原。')" /></div><input id="ocr-merge-lines" v-model="draft.mergeLines" type="checkbox" role="switch" @change="autosave.commit" /></div>
    </section>
    <div v-if="!configuringPreset && !editingServiceId && (autosave.saving.value || autosave.saved.value)" class="save-status" :class="{ 'status-ok': !autosave.saving.value }" role="status"><Check v-if="!autosave.saving.value" :size="14" />{{ autosave.saving.value ? t('正在保存…') : t('已自动保存') }}</div>
    <p v-if="autosave.error.value" class="error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t('重试') }}</Button></p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}</p>
    </div>
  </div>
</template>

<style scoped>
.ocr-settings { font-size:13px; }
.ocr-field { display:grid; grid-template-columns:124px minmax(0,1fr); align-items:center; gap:12px; margin:8px 0; }
.system-status { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin:8px 0; }
.system-status span { overflow-wrap:anywhere; }
.ocr-behavior { margin-top:12px; border-top:1px solid var(--desk-border,#e1e4e8); }
.hint, .save-status { color:var(--desk-muted, #57606a); font-size:12px; line-height:1.6; margin:8px 0; }
.save-status.status-ok { color:var(--ui-success-fg,#24834b); }
.error { color:#b91c1c; overflow-wrap:anywhere; }
</style>
