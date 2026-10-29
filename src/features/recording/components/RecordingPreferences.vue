<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { t } from '@/shared/i18n/i18n'
import SelectControl from '@/shared/ui/SelectControl.vue'
import RecordingFileSettings from './RecordingFileSettings.vue'
import RecordingInputSettings from './RecordingInputSettings.vue'
import { RECORDING_PREFERENCES_KEY } from '../preferences'
import { useRecordingSetup } from '../use-recording-setup'
import { recordingBitrate, type RecorderCapabilities } from '../recording'

defineProps<{ disabled: boolean; capabilities: RecorderCapabilities | null }>()
const { recordingFormat, recordingFps, recordingEncoding, recordingQuality, recordingMbps, recordingCountdown,
  captureCursor, recordingEffects, hideMainWindow, persist, reloadPreferences } = useRecordingSetup()
const message = ref('')
const fileSettings = ref<InstanceType<typeof RecordingFileSettings> | null>(null)
const emit = defineEmits<{ directoryChanged: [] }>()
/** 选区工具条也能修改偏好；远端写入后同步主窗口，避免下次启动回写旧值。 */
function syncPreferences(event: StorageEvent) {
  if (event.key === RECORDING_PREFERENCES_KEY || event.key === null) {
    reloadPreferences(); message.value = ''
  }
}
onMounted(() => { if (typeof window !== 'undefined') window.addEventListener('storage', syncPreferences) })
onUnmounted(() => { if (typeof window !== 'undefined') window.removeEventListener('storage', syncPreferences) })
const invalid = computed(() => { try { if (recordingFormat.value === 'mp4') recordingBitrate(recordingMbps.value); return '' } catch (e) { return (e as Error).message } })
/** 进入选区前同步落盘，让复用的选区窗口读取这次设置。 */
async function save() {
  if (invalid.value || !await fileSettings.value?.flush()) return false
  const saved = persist()
  message.value = saved ? '' : '无法保存录屏偏好，请检查本地存储后重试。'
  return saved
}
defineExpose({ save })
</script>

<template>
  <section class="recording-preferences" @input="message = ''" @change="message = ''">
    <fieldset :disabled="disabled">
      <legend class="section-heading">{{ t('视频与导出') }}</legend>
      <div class="setting-row"><label for="record-format">{{ t('保存格式') }}<small>{{ t('MP4 支持声音，GIF 适合短动画') }}</small></label><SelectControl id="record-format" v-model="recordingFormat" :options="[{ value: 'mp4', label: 'MP4', disabled: !!capabilities?.formats && !capabilities.formats.includes('mp4') }, { value: 'gif', label: 'GIF', disabled: !!capabilities?.formats && !capabilities.formats.includes('gif') }]" /></div>
      <div class="setting-row"><label for="record-fps">{{ t('帧率') }}</label><SelectControl id="record-fps" v-model="recordingFps" :options="(recordingFormat === 'gif' ? [10, 20] : [30, 60]).map(fps => ({ value: fps, label: `${fps} FPS`, disabled: !!capabilities?.frameRates && !capabilities.frameRates.includes(fps) }))" /></div>
      <template v-if="recordingFormat === 'mp4'">
        <div class="setting-row"><label for="record-quality">{{ t('画质') }}</label><SelectControl id="record-quality" v-model="recordingQuality" :disabled="recordingMbps !== ''" :options="[{ value: 'economy', label: t('节省空间') }, { value: 'balanced', label: t('均衡') }, { value: 'high', label: t('高清') }]" /></div>
        <div class="setting-row"><label for="record-bitrate">{{ t('码率（Mbps）') }}<small>{{ t('1–80 Mbps；清空后恢复自动码率') }}</small></label><input id="record-bitrate" v-model="recordingMbps" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :aria-invalid="!!invalid" aria-describedby="record-preference-error" /></div>
        <div class="setting-row"><label for="record-encoding">{{ t('编码方式') }}</label><SelectControl id="record-encoding" v-model="recordingEncoding" :options="[{ value: 'auto', label: t('自动编码') }, { value: 'software', label: t('软件兼容') }]" /></div>
      </template>
      <h2 class="section-heading">{{ t('录制行为') }}</h2>
      <div class="setting-row"><label for="record-countdown">{{ t('倒计时') }}</label><SelectControl id="record-countdown" v-model="recordingCountdown" :options="[0, 3, 5].map(value => ({ value, label: value ? t('{0} 秒', { 0: value }) : t('无延时') }))" /></div>
      <div class="setting-row"><label for="record-cursor">{{ t('录制光标') }}</label><input id="record-cursor" v-model="captureCursor" type="checkbox" :disabled="capabilities?.captureCursor === false" /></div>
      <div class="setting-row"><label for="record-hide">{{ t('录制时收起主界面') }}</label><input id="record-hide" v-model="hideMainWindow" type="checkbox" /></div>
      <p class="settings-note">{{ t('声音在选择范围后开启，每次录制单独确认。') }}</p>
      <div class="recording-effects-settings"><RecordingInputSettings v-model="recordingEffects" :disabled="disabled" /></div>
      <RecordingFileSettings ref="fileSettings" :disabled="disabled" :format="recordingFormat" @changed="emit('directoryChanged')" />
    </fieldset>
    <p v-if="invalid" id="record-preference-error" class="notice error" role="alert">{{ t(invalid) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
  </section>
</template>

<style scoped>
fieldset { margin:0; padding:0; border:0; min-width:0; }
legend { float:left; width:100%; }
legend + div { clear:both; }
.setting-row :deep(.select-trigger), .setting-row input[type=number] { width:180px; }
.settings-note { color:var(--desk-muted); font-size:12px; line-height:1.7; margin:10px 0; }
.recording-effects-settings { margin-top:20px; }
.settings-actions { display:flex; flex-wrap:wrap; align-items:center; gap:12px; padding:16px 0; }
</style>
