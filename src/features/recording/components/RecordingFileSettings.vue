<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api, desktop } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import Button from '@/shared/ui/button/Button.vue'
import { defaultRecordingExportSettings, validateRecordingExportSettings } from '@/features/settings/settings'
import { updateSettings } from '@/features/settings/settings-write'

defineProps<{ disabled: boolean; format: string }>()
const emit = defineEmits<{ changed: [] }>()
const draft = ref(defaultRecordingExportSettings()), ready = ref(false), loadError = ref('')
async function load() {
  if (!desktop) return
  try { draft.value = { ...(await api.status()).settings.recordingExport }; ready.value = true; loadError.value = '' }
  catch (e) { loadError.value = String(e) }
}
const autosave = useAutoSave(async () => {
  const value = { ...draft.value }
  await updateSettings(current => ({ ...current, recordingExport: value }))
  emit('changed')
}, () => validateRecordingExportSettings(draft.value))
async function flush() { await autosave.flush(); return ready.value && !autosave.error.value }
defineExpose({ flush })
onMounted(load)
</script>

<template>
  <section class="recording-file-settings">
    <h2 class="section-heading">{{ t('文件命名') }}</h2>
    <form @submit.prevent @input="autosave.schedule" @change="autosave.commit" @focusout="autosave.flush">
      <fieldset :disabled="disabled || !ready">
        <div class="setting-row"><label for="record-prefix">{{ t('文件名前缀') }}</label><input id="record-prefix" v-model="draft.filenamePrefix" maxlength="48" required /></div>
        <div class="setting-row"><label for="record-dimensions">{{ t('文件名包含视频尺寸') }}<small class="break-all">{{ t('示例：{0}-Recording-时间戳-标识{1}.{2}', { 0: draft.filenamePrefix, 1: draft.includeDimensions ? '-1920x1080' : '', 2: format }) }}</small></label><input id="record-dimensions" v-model="draft.includeDimensions" type="checkbox" /></div>
        <div class="setting-row"><label for="record-directory-memory">{{ t('记住上次保存目录') }}<small>{{ t('关闭后，本次选择仍可使用；下次启动恢复默认目录。') }}</small></label><input id="record-directory-memory" v-model="draft.rememberDirectory" type="checkbox" /></div>
      </fieldset>
    </form>
    <p class="settings-save-status" role="status">{{ autosave.saving.value ? t('正在保存…') : autosave.saved.value ? t('已自动保存') : t('更改自动保存') }}</p>
    <p v-if="loadError || autosave.error.value" class="notice error" role="alert">{{ te(loadError || autosave.error.value) }} <Button variant="ghost" :disabled="disabled" @click="ready ? autosave.retry() : load()">{{ t('重试') }}</Button></p>
  </section>
</template>

<style scoped>
fieldset { margin:0; padding:0; border:0; min-width:0; }
input:not([type=checkbox]) { width:192px; padding:4px 8px; }
</style>
