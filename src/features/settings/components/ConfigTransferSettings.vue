<script setup lang="ts">
import { ref } from 'vue'
import { Check } from 'lucide-vue-next'
import { t, te } from '@/shared/i18n/i18n'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop } from '@/services/api'
import type { Settings } from '@/features/settings/settings'
import { queueSettingsOperation } from '@/features/settings/settings-write'
import '@/features/settings/settings.css'

const props = defineProps<{ prepareTransfer?: () => Promise<boolean> }>()
const busy = ref(false), error = ref(''), message = ref('')
/** 导入导出复用配置队列，先提交其他页面的草稿，再打开原生文件对话框。 */
async function transfer(kind: 'import' | 'export') {
  if (!desktop || busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    if (props.prepareTransfer && !await props.prepareTransfer()) throw new Error('请先修正未保存的设置，再重试。')
    if (kind === 'export') {
      if (await queueSettingsOperation(() => api.exportSettings())) message.value = '配置已导出，包含快捷键和截图、录屏保存目录。'
    } else {
      const settings: Settings | null = await queueSettingsOperation(() => api.importSettings())
      // 原生 settings-imported 事件统一更新主页面；取消选择不显示成功。
      if (settings) message.value = '配置已导入，快捷键和保存目录已更新。'
    }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
</script>

<template>
  <section class="general-settings-page">
    <section class="general-section">
      <p class="text-xs text-zinc-500">{{ t('备份包含主配置、快捷键和保存目录，不包含密钥、历史记录及图片视频。') }}</p>
      <div class="mt-4 flex flex-wrap gap-2">
        <Button variant="outline" :disabled="!desktop || busy" @click="transfer('import')">{{ t('导入配置') }}</Button>
        <Button variant="outline" :disabled="!desktop || busy" @click="transfer('export')">{{ t('导出配置') }}</Button>
      </div>
      <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
      <p v-if="message" class="notice status-ok" role="status"><Check :size="14" />{{ t(message) }}</p>
    </section>
  </section>
</template>
