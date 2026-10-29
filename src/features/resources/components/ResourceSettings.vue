<script lang="ts">
// 页面卸载后保留最近一次确认状态；重新进入先展示快照，再向原生核对文件变化。
let latestResourceStatus: import('@/features/resources/types').ResourceStatus | null = null
let resourceStatusRevision = 0
</script>

<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop, onResourceProgress } from '@/services/api'
import type { ResourceProgress, ResourceStatus } from '@/features/resources/types'

// 两个设置页复用下载、校验和进度逻辑，各自只展示所属资源。
defineProps<{ kind: 'ocr' | 'font' }>()
const status = ref<ResourceStatus | null>(latestResourceStatus)
const busy = ref(false), error = ref('')
const checking = ref(false)
const progress = ref<ResourceProgress | null>(null)
let unlisten: (() => void) | undefined
let disposed = false
async function refresh() {
  const revision = ++resourceStatusRevision
  checking.value = true
  try {
    const next = await api.resourceStatus()
    if (revision === resourceStatusRevision) {
      latestResourceStatus = next
      if (!disposed) status.value = next
    }
  } catch (e) {
    if (revision === resourceStatusRevision) {
      latestResourceStatus = null
      if (!disposed) { status.value = null; error.value = String(e) }
    }
  } finally { if (!disposed) checking.value = false }
}
async function run(action: () => Promise<ResourceStatus>) {
  if (!desktop || busy.value || checking.value) return
  const revision = ++resourceStatusRevision
  busy.value = true; error.value = ''; progress.value = null
  try {
    const next = await action()
    if (revision === resourceStatusRevision) { latestResourceStatus = next; if (!disposed) status.value = next }
  }
  catch (e) {
    error.value = String(e)
    // 部分识别资源可能已成功下载；重新读取完整状态，重试会跳过校验通过的文件。
    await refresh()
  } finally { busy.value = false; progress.value = null }
}
onMounted(() => {
  if (!desktop) return
  // 订阅进度不阻塞首次状态读取，状态查询也不占用下载操作的 busy 状态。
  void onResourceProgress(value => { if (!disposed) progress.value = value }).then(stop => {
    if (disposed) stop(); else unlisten = stop
  }).catch(e => { if (!disposed) error.value = String(e) })
  void refresh()
})
onUnmounted(() => { disposed = true; unlisten?.() })
</script>

<template>
  <section :class="kind === 'font' ? 'settings-card mb-4' : ''" :aria-label="kind === 'ocr' ? t('文字识别资源') : t('导出字体')" :aria-busy="busy || checking">
    <div v-if="kind === 'ocr'" class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>{{ t("OCR 文字识别") }}</strong><small class="block text-zinc-500">{{ t("离线识别，图片不上传。") }}</small></div>
      <span :class="{ 'status-ok': status?.ocrReady }" role="status"><Check v-if="status?.ocrReady" :size="14" />{{ status ? status.ocrReady ? t("已就绪") : t("引擎{0} · 模型{1}", { 0: status.runtimeReady ? t("就绪") : t("未就绪"), 1: status.modelsReady ? t("就绪") : t("未就绪") }) : checking ? t("正在检查…") : t("状态未读取") }}</span>
      <Button variant="outline" :disabled="!desktop || busy || checking" @click="run(() => api.downloadResources('ocr'))">{{ status?.ocrReady ? t("检查识别资源") : t("下载识别资源") }}</Button>
    </div>
    <div v-else class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>{{ t("PDF / 长图导出字体") }}</strong><small class="block text-zinc-500">{{ t("选择有嵌入授权的 TTF，或下载霞鹜文楷（24 MiB）。") }}</small></div>
      <span :class="{ 'status-ok': status?.fontReady }" role="status"><Check v-if="status?.fontReady" :size="14" />{{ status ? status.fontReady ? t("已就绪") : t("未设置") : checking ? t("正在检查…") : t("状态未读取") }}</span>
      <Button variant="outline" :disabled="!desktop || busy || checking" @click="run(api.chooseExportFont)">{{ t("选择字体") }}</Button>
      <Button variant="ghost" :disabled="!desktop || busy || checking" @click="run(() => api.downloadResources('font'))">{{ t("使用下载字体") }}</Button>
    </div>
    <p v-if="busy" class="py-2 text-xs" role="status">{{ progress ? `${progress.name} · ${Math.floor(progress.received / progress.total * 100)}%` : t("正在处理资源…") }}</p>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
  </section>
</template>
