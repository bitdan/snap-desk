<script setup lang="ts">
import { computed } from 'vue'
import { Keyboard, Settings2 } from 'lucide-vue-next'
import { t } from '@/shared/i18n/i18n'
import RecordingInputSettings from '@/features/recording/components/RecordingInputSettings.vue'
import RecordingOptionMenu from '@/features/recording/components/RecordingOptionMenu.vue'
import type { RecordingFormat, RecordingFps, EncodingPreference, InputEffects, RecordingQuality } from '@/features/recording/recording'

/**
 * 录屏准备条只放图标：格式、帧率、画质、码率、编码、倒计时和收起主界面集中在“录屏设置”面板，
 * 键鼠特效单独一个图标；当前取值写进悬浮提示，不在工具行铺文字。
 */
withDefaults(defineProps<{ busy: boolean; formats?: RecordingFormat[]; formatDisabled?: boolean }>(), { formats: () => [] })
const format = defineModel<RecordingFormat>('format', { required: true })
const fps = defineModel<RecordingFps>('fps', { default: 30 })
const encoding = defineModel<EncodingPreference>('encoding', { required: true })
const quality = defineModel<RecordingQuality>('quality', { required: true })
const mbps = defineModel<string | number>('mbps', { required: true })
const countdown = defineModel<0 | 3 | 5>('countdown', { required: true })
const effects = defineModel<InputEffects>('effects', { required: true })
const hideMainWindow = defineModel<boolean>('hideMainWindow', { default: true })
// 只统计监听开关，尺寸、颜色等偏好本身不代表开启了特效。
const effectCount = computed(() => [effects.value.keyboard, effects.value.clicks, effects.value.movement, effects.value.drag, effects.value.wheel].filter(Boolean).length)
const qualities = computed(() => [{ value: 'economy' as const, label: t('节省空间') }, { value: 'balanced' as const, label: t('均衡') }, { value: 'high' as const, label: t('高清') }])
const encodings = computed(() => [{ value: 'auto' as const, label: t('自动编码') }, { value: 'software' as const, label: t('软件兼容') }])
const countdowns = computed(() => ([0, 3, 5] as const).map(value => ({ value, label: value ? t('{0} 秒', { 0: value }) : t('无延时') })))
const frameRates = computed<RecordingFps[]>(() => format.value === 'gif' ? [10, 20] : [30, 60])
/** 设置图标的提示与读屏名称带上当前取值，不打开面板也能确认。 */
const summary = computed(() => [format.value.toUpperCase(), `${fps.value} FPS`,
  ...(format.value === 'mp4' ? [mbps.value === '' ? qualities.value.find(item => item.value === quality.value)?.label ?? '' : `${mbps.value} Mbps`] : []),
  countdowns.value.find(item => item.value === countdown.value)?.label ?? ''].filter(Boolean).join(' · '))
</script>

<template>
  <div class="recording-inline-options" role="group" :aria-label="t('录屏选项')" @pointerdown.stop @contextmenu.stop.prevent>
    <RecordingOptionMenu :icon="Settings2" :label="t('录屏设置')" :status="summary" wide :disabled="busy">
      <div class="setup-grid">
        <template v-if="formats.length > 1">
          <span>{{ t('录屏格式') }}</span>
          <div class="segmented" role="radiogroup" :aria-label="t('录屏格式')">
            <button v-for="value in formats" :key="value" type="button" role="radio" :aria-checked="format === value" :disabled="busy || formatDisabled" @click="format = value">{{ value.toUpperCase() }}</button>
          </div>
        </template>
        <span>{{ t('帧率') }}</span>
        <div class="segmented" role="radiogroup" :aria-label="t('录屏帧率')">
          <button v-for="value in frameRates" :key="value" type="button" role="radio" :aria-checked="fps === value" :disabled="busy" @click="fps = value">{{ value }} FPS</button>
        </div>
        <template v-if="format === 'mp4'">
          <span>{{ t('画质') }}</span>
          <div class="segmented" role="radiogroup" :aria-label="t('画质')">
            <button v-for="item in qualities" :key="item.value" type="button" role="radio" :aria-checked="quality === item.value" :disabled="busy || mbps !== ''" @click="quality = item.value">{{ item.label }}</button>
          </div>
          <span>{{ t('码率（Mbps）') }}</span>
          <label class="bitrate"><input v-model="mbps" type="number" min="1" max="80" step="any" :placeholder="t('自动')" :aria-label="t('码率（Mbps）')" :disabled="busy" /><small>{{ t('1–80 Mbps；清空后恢复自动码率') }}</small></label>
          <span>{{ t('编码方式') }}</span>
          <div class="segmented" role="radiogroup" :aria-label="t('编码方式')">
            <button v-for="item in encodings" :key="item.value" type="button" role="radio" :aria-checked="encoding === item.value" :disabled="busy" @click="encoding = item.value">{{ item.label }}</button>
          </div>
        </template>
        <span>{{ t('倒计时') }}</span>
        <div class="segmented" role="radiogroup" :aria-label="t('倒计时')">
          <button v-for="item in countdowns" :key="item.value" type="button" role="radio" :aria-checked="countdown === item.value" :disabled="busy" @click="countdown = item.value">{{ item.label }}</button>
        </div>
        <span>{{ t('收起主界面') }}</span>
        <button type="button" class="hide-main-switch" role="switch" :aria-checked="hideMainWindow" :aria-label="t('录制时收起主界面')" :disabled="busy" @click="hideMainWindow = !hideMainWindow"><span class="switch-track" aria-hidden="true"><span /></span></button>
      </div>
    </RecordingOptionMenu>
    <RecordingOptionMenu :icon="Keyboard" :label="t('键鼠特效')" :status="effectCount ? t('{0} 项', { 0: effectCount }) : t('已关闭')" :active="effectCount > 0" wide :disabled="busy"><RecordingInputSettings v-model="effects" :disabled="busy" /></RecordingOptionMenu>
  </div>
</template>

<style scoped>
.recording-inline-options { display:inline-flex; align-items:center; gap:2px; }
.setup-grid { display:grid; grid-template-columns:auto minmax(0, 1fr); align-items:center; gap:10px 14px; }
.setup-grid > span { color:var(--tb-muted, #6b7280); font-size:12px; white-space:nowrap; }
.segmented { display:inline-flex; justify-self:start; padding:2px; border-radius:7px; background:var(--tb-hover, #f1f3f5); }
.segmented button { height:26px; padding:0 10px; border:0; border-radius:5px; background:transparent; color:var(--tb-muted, #6b7280); font:12px 'Segoe UI', 'Microsoft YaHei', sans-serif; white-space:nowrap; cursor:pointer; }
.segmented button[aria-checked=true] { background:var(--tb-surface, #fff); color:var(--tb-text, #24292f); box-shadow:0 1px 2px #0f172a26; }
.segmented button:disabled { opacity:.45; cursor:default; }
.bitrate { display:flex; flex-direction:column; align-items:flex-start; gap:4px; margin:0 !important; }
.bitrate input { width:88px; }
.bitrate small { font-size:11px; }
.hide-main-switch { justify-self:start; display:inline-flex; align-items:center; height:26px; padding:0; border:0; background:transparent; cursor:pointer; }
.switch-track { display:inline-flex; align-items:center; width:30px; height:18px; padding:2px; border-radius:9px; background:#9ca3af; transition:background .12s; }
.switch-track > span { width:14px; height:14px; border-radius:50%; background:#fff; box-shadow:0 1px 2px #0003; }
.hide-main-switch[aria-checked=true] .switch-track { justify-content:flex-end; background:#2563eb; }
.hide-main-switch:disabled { opacity:.45; cursor:default; }
:focus-visible { outline:2px solid #2563eb; outline-offset:1px; }
</style>
