<script setup lang="ts">
import SettingRow from '@/shared/ui/SettingRow'
import { computed, onMounted, ref } from 'vue'
import { Check, Pin } from 'lucide-vue-next'
import { api, desktop } from '@/services/api'
import { updateSettings } from '@/features/settings/settings-write'
import { defaultPinAppearance, pinAppearanceStyle, validatePinAppearance } from '../pins'
import { t, te } from '@/shared/i18n/i18n'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import SelectControl from '@/shared/ui/SelectControl.vue'
import Button from '@/shared/ui/button/Button.vue'

const draft = ref(defaultPinAppearance()), ready = ref(false), loadError = ref('')
const previewStyle = computed(() => pinAppearanceStyle(draft.value, 168, 92))
async function load() {
  if (!desktop) return
  try {
    draft.value = { ...(await api.status()).settings.pinAppearance }; ready.value = true; loadError.value = ''
  } catch (e) { loadError.value = String(e) }
}
const autosave = useAutoSave(async () => {
  const value = { ...draft.value }
  await updateSettings(current => ({ ...current, pinAppearance: value }))
}, () => validatePinAppearance(draft.value))
function changed() { autosave.schedule() }
function reset() { draft.value = defaultPinAppearance(); changed(); void autosave.flush() }
onMounted(load)
</script>

<template>
  <section class="pin-appearance-settings" aria-labelledby="pin-appearance-title">
    <h2 id="pin-appearance-title" class="section-heading">{{ t('贴图外观') }}<button type="button" :disabled="!desktop || !ready" @click="reset">{{ t('恢复默认') }}</button></h2>
    <div class="appearance-layout">
      <form class="settings-card" @submit.prevent @input="changed" @change="changed(); autosave.flush()" @focusout="autosave.flush">
        <fieldset :disabled="!desktop || !ready">
          <SettingRow :label="t('阴影')" for="pin-shadow"><SelectControl id="pin-shadow" v-model="draft.shadow" :disabled="!desktop || !ready" :options="[{ value: 'auto', label: t('自动调整') }, { value: 'soft', label: t('柔和') }, { value: 'none', label: t('关闭') }]" @update:model-value="changed(); autosave.flush()" /></SettingRow>
          <SettingRow :label="t('边框颜色')" for="pin-border-color"><span class="appearance-control"><input id="pin-border-color" v-model="draft.borderColor" type="color" /><output for="pin-border-color">{{ draft.borderColor.toUpperCase() }}</output></span></SettingRow>
          <SettingRow :label="t('圆角')" for="pin-corner"><span class="appearance-control"><input id="pin-corner" v-model.number="draft.cornerRadius" type="range" min="0" max="24" step="1" /><output for="pin-corner">{{ draft.cornerRadius }} px</output></span></SettingRow>
        </fieldset>
      </form>
      <div class="appearance-preview" :aria-label="t('贴图外观预览')"><div class="preview-sheet" :style="previewStyle"><Pin :size="20" /><span>{{ t('贴图预览') }}</span><div class="preview-lines"><i /><i /></div></div></div>
    </div>
    <p class="settings-save-status" role="status" v-if="autosave.saving.value || autosave.saved.value" :class="{ 'status-ok': !autosave.saving.value }"><Check v-if="!autosave.saving.value" :size="14" class="status-check" />{{ autosave.saving.value ? t('正在保存…') : t('已自动保存') }}</p>
    <p v-if="loadError || autosave.error.value" class="notice error" role="alert">{{ te(loadError || autosave.error.value) }}<Button variant="ghost" @click="ready ? autosave.retry() : load()">{{ t('重试') }}</Button></p>
  </section>
</template>

<style scoped>
.appearance-layout { display:grid; grid-template-columns:minmax(0,1fr) 200px; gap:16px; align-items:stretch; }
fieldset { padding:0; margin:0; border:0; min-width:0; }
.appearance-control { display:flex; align-items:center; gap:10px; }
.appearance-control input[type=color] { width:30px; height:22px; padding:0; border:0; border-radius:4px; background:transparent; cursor:pointer; }
.appearance-control input[type=range] { width:140px; min-width:0; }
output { min-width:56px; font-size:12px; color:var(--desk-muted); white-space:nowrap; font-variant-numeric:tabular-nums; }
.appearance-preview { display:grid; place-items:center; min-height:132px; border:1px solid var(--desk-border); border-radius:8px; background:var(--desk-surface); }
.preview-sheet { width:150px; height:84px; display:flex; flex-wrap:wrap; align-content:center; align-items:center; gap:8px; padding:14px; background:var(--desk-bg); color:var(--desk-accent-fg); border:1px solid var(--desk-border); transition:box-shadow .15s,border-radius .15s; font-size:12px; }
.preview-lines { display:grid; gap:5px; flex-basis:100%; padding-left:28px; }
.preview-lines i { height:3px; border-radius:2px; background:var(--desk-border); }
.preview-lines i:last-child { width:65%; }
@media (max-width:640px) { .appearance-layout { grid-template-columns:minmax(0,1fr); } }
@media (prefers-reduced-motion:reduce) { .preview-sheet { transition:none; } }
</style>
