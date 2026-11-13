<script setup lang="ts">
import InfoTip from '@/shared/ui/InfoTip.vue'
import { t, te } from '@/shared/i18n/i18n'
import { computed, ref } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'
import { shortcutProbeMessages, type ShortcutProbeReport } from '@/features/settings/shortcut-probe'

defineProps<{ disabled: boolean }>()
const emit = defineEmits<{ testing: [value: boolean] }>()
const running = ref(false), error = ref(''), report = ref<ShortcutProbeReport | null>(null)
const messages = computed(() => report.value ? shortcutProbeMessages(report.value) : [])
async function test() {
  if (running.value) return
  running.value = true; error.value = ''; report.value = null; emit('testing', true)
  try { report.value = await api.probeShortcuts() }
  catch (e) { error.value = String(e) }
  finally { running.value = false; emit('testing', false) }
}
</script>

<template>
  <section :aria-label='t("特殊应用快捷键兼容")'>
    <h2 class="section-heading">{{ t('排查') }}</h2>
    <div class="settings-card">
      <div class="setting-row">
        <span class="flex items-center gap-1">{{ t("快捷键没有反应") }}<InfoTip :text="t('开始后五秒内切换到目标软件并按快捷键，收到按键即结束。')" /></span>
        <Button variant="outline" :disabled="!desktop || disabled || running" @click="test">{{ running ? t("检测中…") : t("开始检测") }}</Button>
        <p v-if="running" class="w-full text-xs text-zinc-500" role="status">{{ t("切换到目标软件并按快捷键；测试不会执行截图或录屏。") }}</p>
        <p v-if="error" class="w-full text-sm text-red-600" role="alert">{{ te(error) }}</p>
        <ul v-if="messages.length" class="w-full list-disc space-y-1 pl-5 text-xs" aria-live="polite"><li v-for="message in messages" :key="message">{{ t(message) }}</li></ul>
      </div>
      <slot />
    </div>
  </section>
</template>
