<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import { ref, watch } from 'vue'
import { appearance, type Appearance } from '@/shared/ui/appearance'
import { t } from '@/shared/i18n/i18n'
import { Check, Monitor, Sun, Moon } from 'lucide-vue-next'

const props = defineProps<{ busy: boolean; saved?: boolean }>()
const emit = defineEmits<{ save: [value: Appearance] }>()
const draft = ref({ ...appearance.value })
const themes = [
  { value: 'system', label: '跟随系统', icon: Monitor },
  { value: 'light', label: '浅色', icon: Sun },
  { value: 'dark', label: '深色', icon: Moon },
] as const
watch(appearance, value => { draft.value = { ...value } })
// 保存结束后以已生效配置同步控件，失败时也不会留下看似已保存的选项。
watch(() => props.busy, busy => { if (!busy) draft.value = { ...appearance.value } })
function save() {
  if (!props.busy) emit('save', { ...draft.value })
}
function chooseTheme(theme: Appearance['theme']) {
  if (props.busy || draft.value.theme === theme) return
  draft.value.theme = theme
  save()
}
</script>

<template>
  <div class="appearance-settings">
    <div class="general-section-heading"><h2>{{ t('外观与语言') }}</h2><span v-if="props.saved && !props.busy" class="status-ok" role="status"><Check :size="14" />{{ t('已保存') }}</span></div>
    <div class="setting-row appearance-row">
      <span id="theme-label">{{ t('主题') }}</span>
      <div class="theme-options" role="group" aria-labelledby="theme-label">
        <button v-for="theme in themes" :key="theme.value" type="button" :aria-pressed="draft.theme === theme.value" :disabled="props.busy" @click="chooseTheme(theme.value)"><component :is="theme.icon" :size="15" aria-hidden="true" />{{ t(theme.label) }}</button>
      </div>
    </div>
    <div class="setting-row appearance-row">
      <label for="language">{{ t('语言') }}</label><SelectControl id="language" v-model="draft.language" class="appearance-language" :disabled="props.busy" :options="[{ value: 'zh-CN', label: t('简体中文') }, { value: 'en', label: 'English' }]" @update:model-value="save" />
    </div>
  </div>
</template>
