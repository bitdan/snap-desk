<script setup lang="ts">
import { t } from '@/lib/i18n'
import RecordingInputSettings from './RecordingInputSettings.vue'
import RecordingOptionMenu from './RecordingOptionMenu.vue'
import type { RecordingFormat, EncodingPreference, InputEffects, RecordingQuality } from '@/lib/recording'

defineProps<{ busy: boolean; format: RecordingFormat }>()
const encoding = defineModel<EncodingPreference>('encoding', { required: true })
const quality = defineModel<RecordingQuality>('quality', { required: true })
const mbps = defineModel<string | number>('mbps', { required: true })
const countdown = defineModel<0 | 3 | 5>('countdown', { required: true })
const effects = defineModel<InputEffects>('effects', { required: true })
</script>

<template>
  <!-- 每项只占一个紧凑下拉入口，自定义值与多选特效在各自菜单中编辑。 -->
  <div class="recording-inline-options" role="group" :aria-label="t('录屏选项')" @pointerdown.stop @contextmenu.stop.prevent>
    <template v-if="format === 'mp4'">
      <select v-model="quality" :disabled="busy || mbps !== ''" :aria-label="t('画质')" :title="t('画质')"><option value="economy">{{ t('节省空间') }}</option><option value="balanced">{{ t('均衡') }}</option><option value="high">{{ t('高清') }}</option></select>
      <RecordingOptionMenu :label="mbps === '' ? t('自动码率') : `${mbps} Mbps`" :disabled="busy"><label>{{ t('码率（Mbps）') }}<input v-model="mbps" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :disabled="busy" /></label><small>{{ t('1–80 Mbps；清空后恢复自动码率') }}</small></RecordingOptionMenu>
      <select v-model="encoding" :disabled="busy" :aria-label="t('编码方式')"><option value="auto">{{ t('自动编码') }}</option><option value="software">{{ t('软件兼容') }}</option></select>
    </template>
    <select v-model="countdown" :disabled="busy" :aria-label="t('倒计时')"><option v-for="value in [0, 3, 5]" :key="value" :value="value">{{ value ? t('{0} 秒', { 0: value }) : t('无延时') }}</option></select>
    <RecordingOptionMenu :label="t('键鼠特效')" :disabled="busy"><RecordingInputSettings v-model="effects" :disabled="busy" /></RecordingOptionMenu>
  </div>
</template>

<style scoped>
.recording-inline-options { display: inline-flex; align-items: center; gap: 4px; }
label { display: inline-flex; align-items: center; gap: 4px; }
select, input { height: 28px; padding: 2px 4px; border: 1px solid #e1e4e8; border-radius: 4px; background: white; color: #24292f; font: inherit; }
input[type=number] { width: 70px; }
:focus-visible { outline: 2px solid #2563eb; outline-offset: -2px; }
</style>
