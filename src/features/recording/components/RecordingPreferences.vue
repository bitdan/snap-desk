<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { t } from '@/shared/i18n/i18n'
import SegmentedControl from '@/shared/ui/SegmentedControl.vue'
import SettingRow from '@/shared/ui/SettingRow'
import SwitchControl from '@/shared/ui/SwitchControl'
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
      <h2 class="section-heading">{{ t('视频与导出') }}</h2>
      <div class="settings-card">
        <SettingRow :label="t('保存格式')"><SegmentedControl v-model="recordingFormat" :label="t('保存格式')" :options="[{ value: 'mp4', label: 'MP4', disabled: !!capabilities?.formats && !capabilities.formats.includes('mp4') }, { value: 'gif', label: 'GIF', disabled: !!capabilities?.formats && !capabilities.formats.includes('gif') }]" /></SettingRow>
        <SettingRow :label="t('帧率')"><SegmentedControl v-model="recordingFps" :label="t('帧率')" :options="(recordingFormat === 'gif' ? [10, 20] : [30, 60]).map(fps => ({ value: fps, label: `${fps} FPS`, disabled: !!capabilities?.frameRates && !capabilities.frameRates.includes(fps) }))" /></SettingRow>
        <template v-if="recordingFormat === 'mp4'">
          <SettingRow :label="t('画质')"><SegmentedControl v-model="recordingQuality" :label="t('画质')" :disabled="recordingMbps !== ''" :options="[{ value: 'economy', label: t('节省空间') }, { value: 'balanced', label: t('均衡') }, { value: 'high', label: t('高清') }]" /></SettingRow>
          <SettingRow :label="t('码率（Mbps）')" for="record-bitrate"><input id="record-bitrate" v-model="recordingMbps" class="w-20 text-right" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :aria-invalid="!!invalid" aria-describedby="record-preference-error" /></SettingRow>
          <SettingRow :label="t('编码方式')"><SegmentedControl v-model="recordingEncoding" :label="t('编码方式')" :options="[{ value: 'auto', label: t('自动编码') }, { value: 'software', label: t('软件兼容') }]" /></SettingRow>
        </template>
      </div>
      <h2 class="section-heading">{{ t('录制行为') }}</h2>
      <div class="settings-card">
        <SettingRow :label="t('倒计时')"><SegmentedControl v-model="recordingCountdown" :label="t('倒计时')" :options="[0, 3, 5].map(value => ({ value, label: value ? t('{0} 秒', { 0: value }) : t('无延时') }))" /></SettingRow>
        <SettingRow :label="t('录制光标')" for="record-cursor"><SwitchControl id="record-cursor" v-model="captureCursor" :disabled="disabled || capabilities?.captureCursor === false" /></SettingRow>
        <SettingRow :label="t('录制时收起主界面')" for="record-hide"><SwitchControl id="record-hide" v-model="hideMainWindow" :disabled="disabled" /></SettingRow>
      </div>
      <RecordingInputSettings v-model="recordingEffects" :disabled="disabled" rows />
      <RecordingFileSettings ref="fileSettings" :disabled="disabled" :format="recordingFormat" @changed="emit('directoryChanged')"><slot name="directory" /></RecordingFileSettings>
    </fieldset>
    <p v-if="invalid" id="record-preference-error" class="notice error" role="alert">{{ t(invalid) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
  </section>
</template>

<style scoped>
fieldset { margin:0; padding:0; border:0; min-width:0; }
</style>
