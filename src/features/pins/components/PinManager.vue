<script setup lang="ts">
import { ImagePlus, Clipboard } from 'lucide-vue-next'
import PinAppearanceSettings from './PinAppearanceSettings.vue'
import { t, te } from '@/shared/i18n/i18n'
import { ref } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'

/** 贴图页只保留创建入口与外观；单张贴图的隐藏、关闭、分组在贴图窗口自身的菜单里操作。 */
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
  <section class="pin-manager">
    <div class="pin-actions"><Button :disabled="!desktop || busy" @click="run(api.pinClipboard)"><Clipboard :size="15" />{{ t('剪贴板贴图') }}</Button><Button variant="outline" :disabled="!desktop || busy" @click="run(api.pinFile)"><ImagePlus :size="15" />{{ t('打开图片贴图') }}</Button></div>
    <div v-if="error" class="notice error" role="alert">{{ te(error) }}</div>
    <PinAppearanceSettings />
  </section>
</template>

<style scoped>
.pin-actions { display:flex; align-items:center; flex-wrap:wrap; gap:8px; }
.notice { margin-top:12px; }
</style>
