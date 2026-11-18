<script lang="ts">
// 只缓存公开的资源状态；页面卸载后重新进入先展示快照，再向原生核对。
let latestLocalStatus: import('@/features/recognition/ai').LocalTranslationStatus | null = null
let localStatusRevision = 0
</script>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import { api } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import type { LocalTranslationStatus } from '@/features/recognition/ai'
import Button from '@/shared/ui/button/Button.vue'

/** CTranslate2 引擎与语言包由用户主动下载或导入，在服务管理中维护。 */
const emit = defineEmits<{ changed: [status: LocalTranslationStatus | null] }>()
const status = ref<LocalTranslationStatus | null>(latestLocalStatus)
const checking = ref(false), busy = ref(false), error = ref(''), message = ref('')
let disposed = false
async function refresh() {
  const revision = ++localStatusRevision
  checking.value = true
  try {
    const next = await api.translationModelStatus()
    if (revision === localStatusRevision) {
      latestLocalStatus = next
      if (!disposed) { status.value = next; emit('changed', next) }
    }
  } catch (e) {
    if (revision === localStatusRevision) {
      latestLocalStatus = null
      if (!disposed) { status.value = null; error.value = String(e); emit('changed', null) }
    }
  } finally { if (!disposed) checking.value = false }
}
async function modelAction(modelId: string, action: 'download' | 'import' | 'delete') {
  if (busy.value || checking.value) return
  const revision = ++localStatusRevision
  busy.value = true; error.value = ''; message.value = modelId === 'runtime' ? '正在安装引擎…' : ''
  try {
    const next = await api.translationModelAction(modelId, action)
    if (revision === localStatusRevision) latestLocalStatus = next
    if (!disposed) { status.value = next; emit('changed', next); message.value = modelId === 'runtime' ? '引擎状态已更新。' : '语言包状态已更新。' }
  }
  catch (e) { message.value = ''; error.value = String(e) }
  finally { busy.value = false }
}
onMounted(refresh)
onUnmounted(() => { disposed = true })
</script>

<template>
  <div class="local-resources">
    <div class="model-row runtime-row">
      <span class="runtime-status" :class="status?.runtimeReady ? 'status-ok' : 'text-xs text-zinc-500'" role="status"><Check v-if="status?.runtimeReady" :size="14" />{{ !status ? checking ? t("正在检查…") : t("引擎状态未读取") : status.runtimeReady ? t("本地引擎已安装") : t("翻译引擎未安装或校验失败") }}</span>
      <div class="model-actions">
        <Button variant="outline" :disabled="busy || checking || !status?.runtimeSupported" @click="modelAction('runtime', 'download')">{{ t("下载引擎") }}</Button>
        <Button variant="outline" :disabled="busy || checking || !status?.runtimeSupported" @click="modelAction('runtime', 'import')">{{ t("导入引擎 ZIP") }}</Button>
      </div>
    </div>
    <p v-if="status && !status.runtimeSupported" class="hint">{{ t("翻译引擎下载和导入目前仅支持 Windows x64。") }}</p>
    <div v-for="pack in status?.models" :key="pack.modelId" class="model-row">
      <span class="model-name">{{ pack.modelId.includes('en-zh') ? t("英语 → 简体中文") : t("中文 → 英语") }}</span>
      <span :class="pack.ready ? 'status-ok' : 'text-xs text-zinc-500'" role="status"><Check v-if="pack.ready" :size="14" />{{ pack.ready ? t("已校验") : t("未安装或校验失败") }}</span>
      <div class="model-actions">
        <Button variant="outline" :disabled="busy || checking || !pack.downloadable" @click="modelAction(pack.modelId, 'download')">{{ t("下载") }}{{ pack.downloadBytes ? `（${Math.ceil(pack.downloadBytes / 1048576)} MiB）` : '' }}</Button>
        <Button variant="outline" :disabled="busy || checking" @click="modelAction(pack.modelId, 'import')">{{ t("导入语言包") }}</Button>
        <Button v-if="pack.revision" variant="ghost" :disabled="busy || checking" @click="modelAction(pack.modelId, 'delete')">{{ t("卸载") }}</Button>
      </div>
    </div>
    <p v-if="message" :class="{ 'status-ok': !busy }" role="status"><Check v-if="!busy" :size="14" />{{ t(message) }}</p>
    <p v-if="error" class="error" role="alert">{{ te(error) }}</p>
  </div>
</template>

<style scoped>
.local-resources { font-size:13px; }
.model-row { display:grid; grid-template-columns:minmax(120px,1fr) minmax(100px,1fr) minmax(0,auto); align-items:center; gap:8px 16px; padding:8px 0; min-height:48px; border-bottom:1px solid var(--desk-sep,#eceef1); }
.model-row:last-of-type { border-bottom:0; }
.runtime-status { grid-column:span 2; }
.model-name { overflow-wrap:anywhere; }
.model-actions { display:flex; align-items:center; justify-content:flex-end; flex-wrap:wrap; gap:6px; }
.status-ok { display:inline-flex; align-items:center; gap:4px; color:var(--ui-success-fg,#24834b); }
.hint { margin:6px 0 0; color:var(--desk-muted,#5f6670); font-size:12px; line-height:1.6; }
.error { color:var(--desk-danger,#b91c1c); overflow-wrap:anywhere; }
@media (max-width:760px) {
  .model-row { grid-template-columns:minmax(0,1fr) auto; }
  .model-actions { grid-column:span 2; justify-content:flex-start; }
}
</style>
