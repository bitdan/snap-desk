<script setup lang="ts">
import { Check } from 'lucide-vue-next'
import SettingRow from '@/shared/ui/SettingRow'
import SwitchControl from '@/shared/ui/SwitchControl'
import { onMounted, ref } from 'vue'
import { api, desktop } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import Button from '@/shared/ui/button/Button.vue'
import { defaultRecordingExportSettings, recordingFilenamePreview, validateRecordingExportSettings } from '@/features/settings/settings'
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
    <h2 class="section-heading">{{ t('文件') }}</h2>
    <div class="settings-card">
      <slot />
      <form @submit.prevent @input="autosave.schedule" @change="autosave.commit" @focusout="autosave.flush">
        <fieldset :disabled="disabled || !ready">
          <SettingRow :label="t('文件名前缀')" for="record-prefix" :hint="t('示例：') + recordingFilenamePreview(draft.filenamePrefix, draft.includeDimensions, format)"><input id="record-prefix" v-model="draft.filenamePrefix" class="w-44" maxlength="48" required /></SettingRow>
          <SettingRow :label="t('文件名包含视频尺寸')" for="record-dimensions"><SwitchControl id="record-dimensions" v-model="draft.includeDimensions" :disabled="disabled || !ready" @change="autosave.commit" /></SettingRow>
          <SettingRow :label="t('记住上次保存目录')" for="record-directory-memory"><SwitchControl id="record-directory-memory" v-model="draft.rememberDirectory" :disabled="disabled || !ready" @change="autosave.commit" /></SettingRow>
        </fieldset>
      </form>
    </div>
    <p class="settings-save-status" role="status" v-if="autosave.saving.value || autosave.saved.value" :class="{ 'status-ok': !autosave.saving.value }"><Check v-if="!autosave.saving.value" :size="14" class="status-check" />{{ autosave.saving.value ? t('正在保存…') : t('已自动保存') }}</p>
    <p v-if="loadError || autosave.error.value" class="notice error" role="alert">{{ te(loadError || autosave.error.value) }} <Button variant="ghost" :disabled="disabled" @click="ready ? autosave.retry() : load()">{{ t('重试') }}</Button></p>
  </section>
</template>

<style scoped>
fieldset { margin:0; padding:0; border:0; min-width:0; }
</style>
