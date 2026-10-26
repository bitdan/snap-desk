<script setup lang="ts">
import { t } from '@/lib/i18n'
import RecordingInputSettings from './RecordingInputSettings.vue'
import type { RecordingFormat, EncodingPreference, InputEffects, RecordingQuality } from '@/lib/recording'

defineProps<{ busy: boolean; format: RecordingFormat }>()
const encoding = defineModel<EncodingPreference>('encoding', { required: true })
const quality = defineModel<RecordingQuality>('quality', { required: true })
const mbps = defineModel<string | number>('mbps', { required: true })
const countdown = defineModel<0 | 3 | 5>('countdown', { required: true })
const effects = defineModel<InputEffects>('effects', { required: true })
</script>

<template>
  <!-- 更多选项直接加入录屏行，不再创建下方分类面板或第二个确认入口。 -->
  <div class="recording-inline-options" role="group" :aria-label="t('录屏选项')" @pointerdown.stop @contextmenu.stop.prevent>
    <template v-if="format === 'mp4'">
      <label>{{ t('画质') }}<select v-model="quality" :disabled="busy || mbps !== ''"><option value="economy">{{ t('节省空间') }}</option><option value="balanced">{{ t('均衡') }}</option><option value="high">{{ t('高清') }}</option></select></label>
      <label>{{ t('码率（Mbps）') }}<input v-model="mbps" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :disabled="busy" /></label>
      <label>{{ t('编码方式') }}<select v-model="encoding" :disabled="busy"><option value="auto">{{ t('自动') }}</option><option value="software">{{ t('软件兼容') }}</option></select></label>
    </template>
    <label>{{ t('倒计时') }}<select v-model="countdown" :disabled="busy"><option v-for="value in [0, 3, 5]" :key="value" :value="value">{{ value ? t('{0} 秒', { 0: value }) : t('无延时') }}</option></select></label>
    <RecordingInputSettings v-model="effects" :disabled="busy" inline />
  </div>
</template>

<style scoped>
.recording-inline-options { display: inline-flex; align-items: center; gap: 8px; }
label { display: inline-flex; align-items: center; gap: 4px; }
select, input { height: 28px; padding: 2px 4px; border: 1px solid #e1e4e8; border-radius: 4px; background: white; color: #24292f; font: inherit; }
input[type=number] { width: 70px; }
:focus-visible { outline: 2px solid #2563eb; outline-offset: -2px; }
</style>
