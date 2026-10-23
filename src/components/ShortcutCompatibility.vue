<script setup lang="ts">
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
  <section class="settings-card border-t" aria-label="特殊应用快捷键兼容">
    <div class="flex flex-wrap items-center justify-between gap-2 py-3">
      <h3 class="text-sm font-semibold">特殊应用兼容</h3>
      <Button variant="outline" :disabled="!desktop || disabled || running" @click="test">{{ running ? '测试中，八秒后结束…' : '测试已保存快捷键（八秒）' }}</Button>
    </div>
    <p class="text-xs text-zinc-500" role="status">{{ running ? '现在切换到 Axure 或目标软件并按已保存的快捷键，停留至测试结束。本轮只测试接收，不执行功能动作。' : 'Windows：测试前结束截图和录屏；开始后切换到目标软件并按键。' }}</p>
    <p v-if="error" class="py-2 text-sm text-red-600" role="alert">{{ error }}</p>
    <ul v-if="messages.length" class="list-disc space-y-2 py-3 pl-5 text-xs" aria-live="polite"><li v-for="message in messages" :key="message">{{ message }}</li></ul>
    <h3 class="pt-4 text-sm font-semibold">替代触发</h3>
    <p class="py-2 text-xs text-zinc-500">托盘菜单可直接截图、贴图、录屏或打开剪贴板历史，不依赖热键。需要切回目标软件时，在截图页选择 3 / 5 秒延时，再点击开始截图；录屏也可通过控制条停止。</p>
    <p class="pb-3 text-xs text-zinc-500">默认使用系统全局快捷键，不安装低层键盘钩子或注入目标进程。禁用后注销热键；不承诺绕过游戏反作弊、受保护桌面或系统安全限制。</p>
  </section>
</template>
