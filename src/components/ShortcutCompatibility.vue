<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, ref } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop } from '@/lib/api'
import { shortcutProbeMessages, type ShortcutProbeReport } from '@/lib/shortcut-probe'

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
  <section class="settings-card border-t" :aria-label='t("特殊应用快捷键兼容")'>
    <div class="flex flex-wrap items-center justify-between gap-2 py-3">
      <h3 class="text-sm font-semibold">{{ t("特殊应用兼容") }}</h3>
      <Button variant="outline" :disabled="!desktop || disabled || running" @click="test">{{ running ? t("测试中，八秒后结束…") : t("测试已保存快捷键（八秒）") }}</Button>
    </div>
    <p class="text-xs text-zinc-500" role="status">{{ running ? t("切换到目标软件并按快捷键；测试不会执行截图或录屏。") : t("Windows：测试前结束截图和录屏；开始后切换到目标软件并按键。") }}</p>
    <p v-if="error" class="py-2 text-sm text-red-600" role="alert">{{ te(error) }}</p>
    <ul v-if="messages.length" class="list-disc space-y-2 py-3 pl-5 text-xs" aria-live="polite"><li v-for="message in messages" :key="message">{{ t(message) }}</li></ul>
    <h3 class="pt-4 text-sm font-semibold">{{ t("替代触发") }}</h3>
    <p class="py-2 text-xs text-zinc-500">{{ t("快捷键不可用时，可使用托盘菜单或延时截图。") }}</p>
  </section>
</template>
