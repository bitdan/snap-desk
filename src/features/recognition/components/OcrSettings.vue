<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/services/api'
import { defaultOcrSettings, type SystemOcrStatus } from '@/features/recognition/ocr'
import { updateSettings } from '@/features/settings/settings-write'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { t, te } from '@/shared/i18n/i18n'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'
import ResourceSettings from '@/features/resources/components/ResourceSettings.vue'

const draft = ref(defaultOcrSettings()), ready = ref(false), error = ref('')
const system = ref<SystemOcrStatus | null>(null), checking = ref(false)
const windows = computed(() => draft.value.provider === 'windows-media')
let disposed = false
const languageOptions = computed(() => {
  const options = [{ value: 'auto', label: t('跟随系统首选语言') }, ...(system.value?.languages ?? []).map(item => ({ value: item.tag, label: `${item.name} (${item.tag})` }))]
  if (!options.some(item => item.value === draft.value.language)) options.push({ value: draft.value.language, label: draft.value.language })
  return options
})
const autosave = useAutoSave(async () => {
  const ocr = { ...draft.value }
  await updateSettings(current => ({ ...current, ocr }))
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
onMounted(() => {
  void api.status().then(value => { if (!disposed) { draft.value = { ...value.settings.ocr }; ready.value = true } }).catch(e => { if (!disposed) error.value = String(e) })
  void refreshSystem()
})
onUnmounted(() => { disposed = true })
</script>

<template>
  <div class="ocr-settings">
    <label class="ocr-field">{{ t('OCR 引擎') }}<SelectControl v-model="draft.provider" :aria-label="t('OCR 引擎')" :disabled="!ready || autosave.saving.value" :options="[{ value: 'local-ppocr', label: 'PaddleOCR' }, { value: 'windows-media', label: t('Windows 系统 OCR'), disabled: !system?.supported }]" @update:model-value="autosave.commit" /></label>
    <template v-if="windows">
      <label class="ocr-field">{{ t('识别语言') }}<SelectControl v-model="draft.language" :aria-label="t('识别语言')" :options="languageOptions" :disabled="!ready || checking || autosave.saving.value || !system?.supported || !!system?.error" @update:model-value="autosave.commit" /></label>
      <div class="system-status"><span role="status">{{ checking ? t('正在检查系统语言包…') : !system ? t('状态未读取') : !system.supported ? t('Windows 系统 OCR 仅支持 Windows。') : system.error ? te(system.error) : system.languages.length ? t('已安装 {0} 个 OCR 语言包', { 0: system.languages.length }) : t('未安装 Windows OCR 语言包，请在 Windows 语言设置中安装。') }}</span><Button variant="outline" :disabled="checking" @click="refreshSystem">{{ t('刷新语言包') }}</Button></div>
      <p class="hint">{{ t('使用系统离线识别，图片不上传；失败时不切换其他引擎。') }}</p>
    </template>
    <ResourceSettings v-else-if="ready" kind="ocr" />
    <div class="save-status" role="status">{{ autosave.saving.value ? t('正在保存…') : autosave.saved.value ? t('已自动保存') : t('更改自动保存') }}</div>
    <p v-if="autosave.error.value" class="error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t('重试') }}</Button></p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}</p>
  </div>
</template>

<style scoped>
.ocr-settings { font-size:13px; }
.ocr-field { display:grid; grid-template-columns:110px minmax(0,300px); align-items:center; gap:12px; margin:8px 0; }
.system-status { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin:8px 0; }
.system-status span { overflow-wrap:anywhere; }
.hint, .save-status { color:var(--desk-muted, #57606a); font-size:12px; line-height:1.6; margin:8px 0; }
.error { color:#b91c1c; overflow-wrap:anywhere; }
</style>
