<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from './ui/button/Button.vue'
import { api, desktop, onResourceProgress, type ResourceProgress, type ResourceStatus } from '@/lib/api'

// 两个设置页复用下载、校验和进度逻辑，各自只展示所属资源。
defineProps<{ kind: 'ocr' | 'font' }>()
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
  <section :class="kind === 'font' ? 'settings-card mb-4' : ''" :aria-label="kind === 'ocr' ? '文字识别资源' : '导出字体'" :aria-busy="busy">
    <div v-if="kind === 'ocr'" class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>OCR 文字识别</strong><small class="block text-zinc-500">离线识别，图片不上传。</small></div>
      <span :class="{ 'status-ok': status?.ocrReady }" role="status"><Check v-if="status?.ocrReady" :size="14" />{{ status ? status.ocrReady ? '已就绪' : `引擎${status.runtimeReady ? '就绪' : '未就绪'} · 模型${status.modelsReady ? '就绪' : '未就绪'}` : '状态未读取' }}</span>
      <Button variant="outline" :disabled="!desktop || busy" @click="run(() => api.downloadResources('ocr'))">{{ status?.ocrReady ? '检查识别资源' : '下载识别资源' }}</Button>
    </div>
    <div v-else class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>PDF / 长图导出字体</strong><small class="block text-zinc-500">选择有嵌入授权的 TTF，或下载霞鹜文楷（24 MiB）。</small></div>
      <span :class="{ 'status-ok': status?.fontReady }" role="status"><Check v-if="status?.fontReady" :size="14" />{{ status ? status.fontReady ? '已就绪' : '未设置' : '状态未读取' }}</span>
      <Button variant="outline" :disabled="!desktop || busy" @click="run(api.chooseExportFont)">选择字体</Button>
      <Button variant="ghost" :disabled="!desktop || busy" @click="run(() => api.downloadResources('font'))">使用下载字体</Button>
    </div>
    <p v-if="busy" class="py-2 text-xs" role="status">{{ progress ? `${progress.name} · ${Math.floor(progress.received / progress.total * 100)}%` : '正在处理资源…' }}</p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
  </section>
</template>
