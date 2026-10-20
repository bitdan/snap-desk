<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { ChevronRight, X } from 'lucide-vue-next'
import RecordingInputSettings from './RecordingInputSettings.vue'
import { recordingBitrate, type RecordingFormat, type RecordingFps, type EncodingPreference, type InputEffects, type RecordingQuality } from '@/lib/recording'

const props = defineProps<{ busy: boolean; error: string; formats: RecordingFormat[]; systemAudioAvailable: boolean; microphoneAvailable: boolean }>()
const emit = defineEmits<{ confirm: []; close: [] }>()
const format = defineModel<RecordingFormat>('format', { required: true })
// 切换格式时选用兼容帧率；共享倒计时、光标和键鼠特效。
watch(format, value => { fps.value = value === 'gif' ? 10 : 30 })
const fps = defineModel<RecordingFps>('fps', { required: true })
const encoding = defineModel<EncodingPreference>('encoding', { required: true })
const quality = defineModel<RecordingQuality>('quality', { required: true })
const mbps = defineModel<string | number>('mbps', { required: true })
const countdown = defineModel<0 | 3 | 5>('countdown', { required: true })
const cursor = defineModel<boolean>('cursor', { required: true })
const systemAudio = defineModel<boolean>('systemAudio', { required: true })
const microphone = defineModel<boolean>('microphone', { required: true })
const effects = defineModel<InputEffects>('effects', { required: true })
const groups = [{ id: 'format', label: '输出格式' }, { id: 'quality', label: '画质与码率' }, { id: 'fps', label: '帧率' }, { id: 'encoding', label: '编码方式' }, { id: 'audio', label: '声音' }, { id: 'countdown', label: '倒计时' }, { id: 'cursor', label: '鼠标指针' }, { id: 'effects', label: '键鼠特效' }]
const active = ref('format')
const root = ref<HTMLElement | null>(null)
const bitrateInput = ref<HTMLInputElement | null>(null)
const invalid = ref('')
function select(id: string) { if (!props.busy) active.value = id }
function navigate(event: KeyboardEvent, index: number) {
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? groups.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + groups.length) % groups.length
  select(groups[next]!.id); root.value?.querySelectorAll<HTMLButtonElement>('[role=tab]')[next]?.focus()
}
function confirm() {
  if (props.busy) return
  invalid.value = ''
  try {
    if (format.value === 'mp4' && bitrateInput.value && !bitrateInput.value.validity.valid) throw new Error('码率需为 1–80 Mbps，留空使用自动码率。')
    if (format.value === 'mp4') recordingBitrate(mbps.value)
    emit('confirm')
  } catch (e) { active.value = 'quality'; invalid.value = String(e) }
}
function trapFocus(event: KeyboardEvent) {
  if (event.key !== 'Tab') return
  const items = [...(root.value?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled)') ?? [])].filter(el => el.getClientRects().length)
  const first = items[0], last = items.at(-1)
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
}
onMounted(() => { if (!props.formats.includes(format.value) && props.formats[0]) format.value = props.formats[0] })
onMounted(() => root.value?.querySelector<HTMLButtonElement>('[role=tab]')?.focus())
</script>

<template>
  <section ref="root" class="recording-menu" role="dialog" aria-modal="true" aria-label="录屏选项" @keydown="trapFocus" @pointerdown.stop @pointermove.stop @contextmenu.stop.prevent>
    <header><strong>录屏选项</strong><button title="关闭录屏选项" aria-label="关闭录屏选项" :disabled="busy" @click="emit('close')"><X :size="16" /></button></header>
    <div class="menu-body">
      <nav role="tablist" aria-label="录屏设置分类" aria-orientation="vertical">
        <button v-for="(group, index) in groups" :id="`recording-tab-${group.id}`" :key="group.id" role="tab" :aria-selected="active === group.id" :aria-controls="`recording-option-${group.id}`" :disabled="busy" @mouseenter="select(group.id)" @click="select(group.id)" @keydown="navigate($event, index)">{{ group.label }}<ChevronRight :size="13" /></button>
      </nav>
      <div class="menu-panels">
        <section v-show="active === 'format'" id="recording-option-format" role="tabpanel" aria-labelledby="recording-tab-format">
          <label v-for="value in formats" :key="value"><input v-model="format" type="radio" :value="value" :disabled="busy" />{{ value === 'gif' ? 'GIF 动图' : 'MP4 视频' }}</label>
          <small>GIF 循环播放，适合短片段演示</small>
        </section>
        <section v-show="active === 'quality' && format === 'mp4'" id="recording-option-quality" role="tabpanel" aria-labelledby="recording-tab-quality">
          <label v-for="[value, label] in [['economy', '节省空间'], ['balanced', '均衡'], ['high', '高清']]" :key="value"><input v-model="quality" type="radio" name="recording-quality" :value="value" :disabled="busy || mbps !== ''" />{{ label }}</label>
          <label class="bitrate">码率（Mbps）<input ref="bitrateInput" v-model="mbps" type="number" min="1" max="80" step="any" placeholder="自动" :disabled="busy" title="1–80 Mbps；清空后恢复自动码率" /></label>
          <small>留空使用画质预设</small>
        </section>
        <section v-show="active === 'fps'" id="recording-option-fps" role="tabpanel" aria-labelledby="recording-tab-fps"><label v-for="value in (format === 'gif' ? [10, 20] : [30, 60])" :key="value"><input v-model="fps" type="radio" name="recording-fps" :value="value" :disabled="busy" />{{ value }} FPS</label></section>
        <section v-show="active === 'encoding' && format === 'mp4'" id="recording-option-encoding" role="tabpanel" aria-labelledby="recording-tab-encoding"><label><input v-model="encoding" type="radio" value="auto" :disabled="busy" />自动</label><label><input v-model="encoding" type="radio" value="software" :disabled="busy" />软件兼容</label><small>自动模式优先尝试硬件编码</small></section>
        <section v-show="active === 'countdown'" id="recording-option-countdown" role="tabpanel" aria-labelledby="recording-tab-countdown"><label v-for="value in [0, 3, 5]" :key="value"><input v-model="countdown" type="radio" :value="value" :disabled="busy" />{{ value ? `${value} 秒` : '立即开始' }}</label></section>
        <section v-show="active === 'cursor'" id="recording-option-cursor" role="tabpanel" aria-labelledby="recording-tab-cursor"><label><input v-model="cursor" type="checkbox" :disabled="busy" />录制鼠标指针</label></section>
        <section v-show="active === 'audio'" id="recording-option-audio" role="tabpanel" aria-labelledby="recording-tab-audio">
          <template v-if="format === 'mp4'">
            <label><input v-model="systemAudio" type="checkbox" :disabled="busy || !systemAudioAvailable" />系统声音</label>
            <label><input v-model="microphone" type="checkbox" :disabled="busy || !microphoneAvailable" />麦克风</label>
            <small v-if="systemAudioAvailable || microphoneAvailable">使用默认设备；仅在录制中采集声音，暂停时停止采集。</small>
            <small v-else>当前系统的音频录制不可用。</small>
          </template>
          <small v-else>GIF 不支持声音。</small>
        </section>
        <section v-show="active === 'effects'" id="recording-option-effects" role="tabpanel" aria-labelledby="recording-tab-effects"><RecordingInputSettings v-model="effects" :disabled="busy" /></section>
        <small v-if="format === 'gif' && ['quality', 'encoding'].includes(active)">GIF 使用软件调色板编码，无需设置视频码率。</small>
      </div>
    </div>
    <p v-if="error || invalid" class="menu-error" role="alert">{{ error || invalid }}</p>
    <footer><small>{{ format.toUpperCase() }} · {{ format === 'mp4' && (systemAudio || microphone) ? '有声' : '无声' }} · {{ fps }} FPS</small><button class="confirm" :disabled="busy" title="确认选区后，在控制条开始录制" @click="confirm">{{ busy ? '准备中…' : '确认选区' }}</button></footer>
  </section>
</template>

<style scoped>
.recording-menu { position: fixed; width: min(390px, calc(100vw - 16px)); max-height: calc(100vh - 16px); display: flex; flex-direction: column; overflow: auto; border: 1px solid #e4e4e7; border-radius: 9px; background: white; color: #27272a; font-size: 12px; box-shadow: 0 8px 32px #0003; }
header, footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 10px 12px; flex-shrink: 0; }
header { border-bottom: 1px solid #eee; } header button { display: flex; padding: 4px; border-radius: 4px; }
.menu-body { display: grid; grid-template-columns: 120px minmax(0, 1fr); min-height: 230px; overflow-y: auto; }
nav { padding: 6px; border-right: 1px solid #eee; }
nav button { display: flex; justify-content: space-between; align-items: center; width: 100%; padding: 9px 6px; border-radius: 5px; }
button[aria-selected=true], button:hover:not(:disabled) { background: #f1edff; color: #7c3aed; }
.menu-panels { padding: 12px; max-height: min(330px, 55vh); overflow-y: auto; }
.menu-panels label { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.menu-panels .bitrate { flex-wrap: wrap; margin-top: 16px; margin-bottom: 6px; }
input[type=number] { width: 90px; border: 1px solid #d4d4d8; border-radius: 4px; padding: 5px; }
small { color: #71717a; font-size: 11px; } footer { border-top: 1px solid #eee; }
.confirm { background: #7c3aed; color: white; border-radius: 5px; padding: 7px 12px; }
.menu-error { margin: 0; padding: 8px 12px; color: #be123c; background: #fff1f2; }
button:disabled { opacity: .5; } :focus-visible { outline: 2px solid #a78bfa; outline-offset: 1px; }
</style>
