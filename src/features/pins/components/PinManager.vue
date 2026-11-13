<script setup lang="ts">
import { Pin } from 'lucide-vue-next'
import ToolHero from '@/shared/ui/ToolHero.vue'
import PinAppearanceSettings from './PinAppearanceSettings.vue'
import { t, te } from '@/shared/i18n/i18n'
import { ref } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'

/** 贴图页只保留创建入口与外观；单张贴图的隐藏、关闭、分组在贴图窗口自身的菜单里操作。 */
defineProps<{ shortcut?: string }>()
const busy = ref(false)
const error = ref('')
async function run(task: () => Promise<unknown>) {
  if (busy.value) return
  busy.value = true; error.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
</script>

<template>
  <section class="pin-page">
    <ToolHero :icon="Pin" :title="t('剪贴板贴图')" :shortcut="shortcut">
      <Button variant="outline" :disabled="!desktop || busy" @click="run(api.pinFile)">{{ t('打开图片贴图') }}</Button>
      <Button :disabled="!desktop || busy" @click="run(api.pinClipboard)">{{ t('剪贴板贴图') }}</Button>
    </ToolHero>
    <div v-if="error" class="notice error" role="alert">{{ te(error) }}</div>
    <PinAppearanceSettings />
  </section>
</template>

<style scoped>
.notice { margin-top:12px; }
</style>
