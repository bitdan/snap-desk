<script setup lang="ts">
import { ref, watch } from 'vue'
import { appearance, type Appearance } from '@/shared/ui/appearance'
import { t } from '@/shared/i18n/i18n'
import Button from '@/shared/ui/button/Button.vue'

const props = defineProps<{ busy: boolean }>()
const emit = defineEmits<{ save: [value: Appearance] }>()
const draft = ref({ ...appearance.value })
watch(appearance, value => { draft.value = { ...value } })
</script>

<template>
  <form class="settings-card mb-4" @submit.prevent="emit('save', { ...draft })">
    <h2 class="section-heading">{{ t('外观与语言') }}</h2>
    <div class="setting-row"><label for="theme">{{ t('主题') }}</label><select id="theme" v-model="draft.theme" :disabled="props.busy"><option value="system">{{ t('跟随系统') }}</option><option value="light">{{ t('浅色') }}</option><option value="dark">{{ t('深色') }}</option></select></div>
    <div class="setting-row"><label for="language">{{ t('语言') }}</label><select id="language" v-model="draft.language" :disabled="props.busy"><option value="zh-CN">{{ t("简体中文") }}</option><option value="en">English</option></select></div>
    <div class="py-3"><Button type="submit" :disabled="props.busy">{{ t('保存设置') }}</Button></div>
  </form>
</template>
