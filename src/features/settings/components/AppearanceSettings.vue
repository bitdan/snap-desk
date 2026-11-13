<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import SegmentedControl from '@/shared/ui/SegmentedControl.vue'
import SettingRow from '@/shared/ui/SettingRow'
import { ref, watch } from 'vue'
import { appearance, type Appearance } from '@/shared/ui/appearance'
import { t } from '@/shared/i18n/i18n'
import { Check } from 'lucide-vue-next'

const props = defineProps<{ busy: boolean; saved?: boolean }>()
const emit = defineEmits<{ save: [value: Appearance] }>()
const draft = ref({ ...appearance.value })
const themes = [
  { value: 'system', label: '跟随系统' },
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
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
  <section class="appearance-settings">
    <h2 class="section-heading">{{ t('外观与语言') }}<span v-if="props.saved && !props.busy" class="status-ok" role="status"><Check :size="14" />{{ t('已保存') }}</span></h2>
    <div class="settings-card">
      <SettingRow :label="t('主题')"><SegmentedControl :model-value="draft.theme" :label="t('主题')" :disabled="props.busy" :options="themes.map(theme => ({ value: theme.value, label: t(theme.label) }))" @update:model-value="chooseTheme" /></SettingRow>
      <SettingRow :label="t('语言')" for="language"><SelectControl id="language" v-model="draft.language" :disabled="props.busy" :options="[{ value: 'zh-CN', label: t('简体中文') }, { value: 'en', label: 'English' }]" @update:model-value="save" /></SettingRow>
    </div>
  </section>
</template>
