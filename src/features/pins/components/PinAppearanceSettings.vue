<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Pin, RotateCcw } from 'lucide-vue-next'
import { api, desktop } from '@/services/api'
import { updateSettings } from '@/features/settings/settings-write'
import { defaultPinAppearance, pinAppearanceStyle, validatePinAppearance } from '../pins'
import type { PinAppearance } from '../types'
import { t, te } from '@/shared/i18n/i18n'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'

const emit = defineEmits<{ preview: [value: PinAppearance] }>()
const draft = ref(defaultPinAppearance()), ready = ref(false), loadError = ref('')
const previewStyle = computed(() => pinAppearanceStyle(draft.value, 168, 92))
async function load() {
  if (!desktop) return
  try {
    draft.value = { ...(await api.status()).settings.pinAppearance }; ready.value = true; loadError.value = ''
    emit('preview', { ...draft.value })
  } catch (e) { loadError.value = String(e) }
}
const autosave = useAutoSave(async () => {
  const value = { ...draft.value }
  await updateSettings(current => ({ ...current, pinAppearance: value }))
}, () => validatePinAppearance(draft.value))
function changed() { emit('preview', { ...draft.value }); autosave.schedule() }
function reset() { draft.value = defaultPinAppearance(); changed(); void autosave.flush() }
onMounted(load)
</script>

<template>
  <section class="pin-appearance-settings" aria-labelledby="pin-appearance-title">
    <div class="appearance-heading"><h2 id="pin-appearance-title">{{ t('贴图外观') }}</h2><Button variant="ghost" :disabled="!desktop || !ready" @click="reset"><RotateCcw :size="14" />{{ t('恢复默认') }}</Button></div>
    <div class="appearance-layout">
      <form class="appearance-fields" @submit.prevent @input="changed" @change="changed(); autosave.flush()" @focusout="autosave.flush">
        <fieldset :disabled="!desktop || !ready">
          <div class="setting-row"><label for="pin-shadow">{{ t('阴影') }}<small>{{ t('自动按贴图尺寸、缩放与焦点调整。') }}</small></label><SelectControl id="pin-shadow" v-model="draft.shadow" :disabled="!desktop || !ready" :options="[{ value: 'auto', label: t('自动调整') }, { value: 'soft', label: t('柔和') }, { value: 'none', label: t('关闭') }]" @update:model-value="changed(); autosave.flush()" /></div>
          <div class="setting-row"><label for="pin-border-color">{{ t('边框颜色') }}</label><div class="border-color-control"><input id="pin-border-color" v-model="draft.borderColor" type="color" /><output for="pin-border-color">{{ draft.borderColor.toUpperCase() }}</output></div></div>
          <div class="setting-row"><label for="pin-corner">{{ t('圆角') }}<small>{{ t('0 为直角') }}</small></label><div class="corner-control"><input id="pin-corner" v-model.number="draft.cornerRadius" type="range" min="0" max="24" step="1" /><output for="pin-corner">{{ draft.cornerRadius }} px</output></div></div>
        </fieldset>
        <p class="settings-save-status" role="status">{{ autosave.saving.value ? t('正在保存…') : autosave.saved.value ? t('已自动保存') : t('应用于全部贴图，修改自动生效。') }}</p>
      </form>
      <div class="appearance-preview" :aria-label="t('贴图外观预览')"><div class="preview-sheet" :style="previewStyle"><Pin :size="20" /><span>{{ t('贴图预览') }}</span><div class="preview-lines"><i /><i /></div></div></div>
    </div>
    <p v-if="loadError || autosave.error.value" class="notice error" role="alert">{{ te(loadError || autosave.error.value) }}<Button variant="ghost" @click="ready ? autosave.retry() : load()">{{ t('重试') }}</Button></p>
  </section>
</template>

<style scoped>
.pin-appearance-settings { margin-top:24px; padding-top:16px; border-top:1px solid var(--desk-border); }
.appearance-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; }
h2 { margin:0; font-size:14px; font-weight:600; }
.appearance-layout { display:grid; grid-template-columns:minmax(0,1fr) 216px; gap:32px; align-items:center; }
fieldset { padding:0; margin:0; border:0; min-width:0; }
.border-color-control { display:flex; align-items:center; gap:12px; width:180px; max-width:100%; }
.border-color-control input[type=color] { width:36px; height:28px; padding:3px; border:1px solid var(--desk-border); border-radius:6px; background:var(--desk-bg); cursor:pointer; }
.corner-control { display:flex; align-items:center; gap:12px; width:180px; max-width:100%; }
input[type=range] { width:132px; min-width:0; accent-color:#3264df; }
output { font-size:12px; color:var(--desk-muted); white-space:nowrap; font-variant-numeric:tabular-nums; }
.appearance-preview { display:grid; place-items:center; min-height:148px; border:1px solid var(--desk-border); border-radius:8px; background:var(--desk-soft); }
.preview-sheet { width:168px; height:92px; display:flex; flex-wrap:wrap; align-content:center; align-items:center; gap:8px; padding:16px; background:var(--desk-bg); color:var(--desk-accent); border:1px solid var(--desk-border); transition:box-shadow .15s,border-radius .15s; font-size:12px; }
.preview-lines { display:grid; gap:5px; flex-basis:100%; padding-left:28px; }
.preview-lines i { height:3px; border-radius:2px; background:var(--desk-border); }
.preview-lines i:last-child { width:65%; }
@media (max-width:720px) { .appearance-layout { grid-template-columns:minmax(0,1fr); gap:12px; } .appearance-preview { min-height:128px; } }
@media (prefers-reduced-motion:reduce) { .preview-sheet { transition:none; } }
</style>
