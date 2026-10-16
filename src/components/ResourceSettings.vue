<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import { api, desktop, onResourceProgress, type ResourceProgress, type ResourceStatus } from '@/lib/api'

const status = ref<ResourceStatus | null>(null)
const busy = ref(false), error = ref('')
const progress = ref<ResourceProgress | null>(null)
let unlisten: (() => void) | undefined
let disposed = false
async function run(action: () => Promise<ResourceStatus>) {
  if (!desktop || busy.value) return
  busy.value = true; error.value = ''; progress.value = null
  try { status.value = await action() }
  catch (e) {
    error.value = String(e)
    // 部分识别资源可能已成功下载；重新读取完整状态，重试会跳过校验通过的文件。
    try { status.value = await api.resourceStatus() } catch { status.value = null }
  } finally { busy.value = false; progress.value = null }
}
onMounted(async () => {
  if (!desktop) return
  try {
    const stop = await onResourceProgress(value => { progress.value = value })
    if (disposed) stop(); else unlisten = stop
  } catch (e) { error.value = String(e) }
  if (!disposed) await run(api.resourceStatus)
})
onUnmounted(() => { disposed = true; unlisten?.() })
</script>

<template>
  <section class="settings-card mb-4" aria-label="文字识别与导出字体" :aria-busy="busy">
    <div class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>OCR 文字识别</strong><small class="block text-zinc-500">PP-OCRv5 中英文模型约 21 MiB<span v-if="status?.runtimeDownloadMib">，引擎下载约 {{ status.runtimeDownloadMib }} MiB</span>。下载后离线识别，图片不上传。</small></div>
      <span role="status">{{ status ? status.ocrReady ? '已就绪' : `引擎${status.runtimeReady ? '就绪' : '未就绪'} · 模型${status.modelsReady ? '就绪' : '未就绪'}` : '状态未读取' }}</span>
      <Button variant="outline" :disabled="!desktop || busy" @click="run(() => api.downloadResources('ocr'))">{{ status?.ocrReady ? '检查识别资源' : '下载识别资源' }}</Button>
    </div>
    <div class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>PDF / 长图导出字体</strong><small class="block text-zinc-500">可下载霞鹜文楷（约 24 MiB，SIL OFL），或选择有嵌入授权的静态 TTF。纯图片导出无需字体。</small></div>
      <span role="status">{{ status ? status.fontReady ? '已就绪' : '未设置' : '状态未读取' }}</span>
      <Button variant="outline" :disabled="!desktop || busy" @click="run(api.chooseExportFont)">选择字体</Button>
      <Button variant="ghost" :disabled="!desktop || busy" @click="run(() => api.downloadResources('font'))">使用下载字体</Button>
    </div>
    <p v-if="status" class="py-2 text-xs text-zinc-500 break-all">资源目录：{{ status.directory }}。字体选择后会复制到此目录；下载字体会替换当前字体。</p>
    <p v-if="busy" class="py-2 text-xs" role="status">{{ progress ? `${progress.name} · ${Math.floor(progress.received / progress.total * 100)}%` : '正在处理资源…' }}</p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
  </section>
</template>
