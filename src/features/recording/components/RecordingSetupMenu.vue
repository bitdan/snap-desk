<script setup lang="ts">
import { computed } from 'vue'
import { t } from '@/shared/i18n/i18n'
import RecordingInputSettings from '@/features/recording/components/RecordingInputSettings.vue'
import RecordingOptionMenu from '@/features/recording/components/RecordingOptionMenu.vue'
import type { RecordingFormat, EncodingPreference, InputEffects, RecordingQuality } from '@/features/recording/recording'

defineProps<{ busy: boolean; format: RecordingFormat }>()
const encoding = defineModel<EncodingPreference>('encoding', { required: true })
const quality = defineModel<RecordingQuality>('quality', { required: true })
const mbps = defineModel<string | number>('mbps', { required: true })
const countdown = defineModel<0 | 3 | 5>('countdown', { required: true })
const effects = defineModel<InputEffects>('effects', { required: true })
// 只统计监听开关，尺寸、颜色等偏好本身不代表开启了特效。
const effectCount = computed(() => [effects.value.keyboard, effects.value.clicks, effects.value.movement, effects.value.drag, effects.value.wheel].filter(Boolean).length)
</script>

<template>
  <!-- 设置入口常驻；画质和编码按组排列，自定义值与特效在各自菜单中编辑。 -->
  <div class="recording-inline-options" role="group" :aria-label="t('录屏选项')" @pointerdown.stop @contextmenu.stop.prevent>
    <template v-if="format === 'mp4'">
      <select v-model="quality" :disabled="busy || mbps !== ''" :aria-label="t('画质')" :title="t('画质')"><option value="economy">{{ t('节省空间') }}</option><option value="balanced">{{ t('均衡') }}</option><option value="high">{{ t('高清') }}</option></select>
      <RecordingOptionMenu :label="mbps === '' ? t('自动码率') : `${mbps} Mbps`" :disabled="busy"><label>{{ t('码率（Mbps）') }}<input v-model="mbps" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :disabled="busy" /></label><small>{{ t('1–80 Mbps；清空后恢复自动码率') }}</small></RecordingOptionMenu>
      <select v-model="encoding" :disabled="busy" :aria-label="t('编码方式')"><option value="auto">{{ t('自动编码') }}</option><option value="software">{{ t('软件兼容') }}</option></select>
    </template>
    <select v-model="countdown" :disabled="busy" :aria-label="t('倒计时')"><option v-for="value in [0, 3, 5]" :key="value" :value="value">{{ value ? t('{0} 秒', { 0: value }) : t('无延时') }}</option></select>
    <RecordingOptionMenu :label="t('键鼠特效')" :status="effectCount ? t('{0} 项', { 0: effectCount }) : t('已关闭')" :active="effectCount > 0" wide :disabled="busy"><RecordingInputSettings v-model="effects" :disabled="busy" /></RecordingOptionMenu>
  </div>
</template>

<style scoped>
.recording-inline-options { display:inline-flex; flex-wrap:wrap; align-items:center; gap:3px; max-width:100%; padding:2px; border:1px solid #e1e4e8; border-radius:5px; background:#f6f7f8; }
.recording-inline-options > * { flex-shrink:0; }
.recording-inline-options :deep(.recording-option-trigger) { padding:3px 6px; border-radius:4px; }
label { display: inline-flex; align-items: center; gap: 4px; }
select, input { height:28px; padding:2px 5px; border:0; border-radius:4px; background:transparent; color:inherit; font:inherit; }
select { cursor:pointer; }
select:hover:enabled { background:#e9edf2; }
:global([data-theme=dark]) .recording-inline-options { background:#272e37; border-color:#444c56; }
:global([data-theme=dark]) select:hover:enabled { background:#303f56; }
input[type=number] { width: 70px; }
:focus-visible { outline: 2px solid #2563eb; outline-offset: -2px; }
</style>
