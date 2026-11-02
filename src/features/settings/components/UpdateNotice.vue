<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { Check, X } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import { api, desktop } from '@/services/api'
import { onAppUpdate, onUpdateWindowRequested } from '@/features/settings/events'
import { updatePreview } from '@/features/settings/update-preview'
import type { AppUpdateView } from '@/features/settings/types'
import { t, te } from '@/shared/i18n/i18n'

const emit = defineEmits<{ version: [version: string | null] }>()
withDefaults(defineProps<{ showTrigger?: boolean }>(), { showTrigger: true })
const view = ref<AppUpdateView | null>(null)
const windowOpen = ref(false), busy = ref(false), error = ref('')
const dialog = ref<{ focus: () => void } | null>(null)
let disposed = false, refreshing = false
let timer: ReturnType<typeof setInterval> | undefined, unsubscribe: (() => void) | undefined
const visible = computed(() => view.value?.availableVersion && view.value.supported && view.value.configured)
function openWindow() { windowOpen.value = true; void nextTick(() => dialog.value?.focus()) }
const offRequest = onUpdateWindowRequested(openWindow)
const progress = computed(() => view.value?.total ? Math.min(100, Math.floor(view.value.downloaded * 100 / view.value.total)) : null)
const operating = computed(() => busy.value || ['checking', 'downloading', 'installing'].includes(view.value?.phase ?? ''))
function receive(next: AppUpdateView) {
  if (disposed) return
  view.value = next
  emit('version', next.availableVersion)
}
async function refresh() {
  if (disposed || refreshing) return
  refreshing = true
  try { receive(await api.appUpdateStatus()) }
  catch { /* 后台查询失败不打断工具操作；操作失败在提示中显示。 */ }
  finally { refreshing = false }
}
async function run(action: () => Promise<AppUpdateView | void>) {
  if (operating.value) return
  busy.value = true; error.value = ''
  try { const next = await action(); if (next) receive(next) }
  catch (e) { if (!disposed) error.value = String(e) }
  finally { busy.value = false; await refresh() }
}
onMounted(async () => {
  if (!desktop && !updatePreview) return
  try {
    unsubscribe = await onAppUpdate(receive)
    if (disposed) { unsubscribe(); return }
  } catch { /* 监听失败时仍通过轮询获取状态。 */ }
  await refresh()
  // 原生下载只在完成时发事件；轮询使顶部提示也能显示中间进度。
  if (!disposed) timer = setInterval(() => { void refresh() }, 1000)
})
onUnmounted(() => { disposed = true; offRequest(); unsubscribe?.(); if (timer) clearInterval(timer) })
</script>

<template>
  <div v-if="showTrigger && visible" class="update-trigger"><Button variant="outline" @click="openWindow"><span class="update-dot" aria-hidden="true" />{{ t('有更新') }}</Button></div>
  <DetachedWindow ref="dialog" :open="windowOpen" :title="t('应用更新')" :width="480" :height="260" @close="windowOpen = false">
  <section v-if="view" class="update-notice" :aria-label="t('应用更新')">
    <div class="update-line" role="status">
      <span class="update-copy"><Check v-if="['ready', 'latest'].includes(view.phase)" :size="14" class="status-check" />{{ t(view.phase === 'latest' ? '当前渠道暂无更新' : view.phase === 'ready' ? '更新已下载并通过校验' : view.phase === 'downloading' ? '正在下载更新…' : view.phase === 'installing' ? '正在退出并安装…' : '发现新版本') }}<template v-if="view.availableVersion"> · v{{ view.availableVersion }}</template><small v-if="updatePreview">{{ t('模拟更新') }}</small></span>
      <span v-if="view.phase === 'downloading'">{{ progress === null ? `${(view.downloaded / 1048576).toFixed(1)} MiB` : `${progress}%` }}</span>
      <Button v-if="['available', 'error'].includes(view.phase)" :disabled="operating" @click="run(api.downloadAppUpdate)">{{ t('下载更新') }}</Button>
      <Button v-if="view.phase === 'ready'" :disabled="operating" @click="run(api.installAppUpdate)">{{ t('退出并安装') }}</Button>
      <Button variant="ghost" :aria-label="t('关闭')" @click="windowOpen = false"><X :size="14" /></Button>
    </div>
    <progress v-if="view.phase === 'downloading'" :value="progress ?? undefined" max="100" :aria-label="t('下载进度')" />
    <p v-if="view.phase === 'ready'" class="update-hint">{{ t(updatePreview ? '模拟更新，不会下载文件或退出应用。' : '安装会退出应用，请先保存工作并结束录屏。') }}</p>
    <p v-if="error || view.error" class="update-error" role="alert">{{ te(error || view.error) }}</p>
  </section>
  </DetachedWindow>
</template>

<style scoped>
.update-trigger { display:flex; justify-content:flex-end; margin-bottom:12px; }
.update-dot { width:6px; height:6px; flex:none; border-radius:50%; background:#dc2626; }
.update-notice { padding:20px; font-size:13px; }
.update-line { display:flex; align-items:center; flex-wrap:wrap; gap:8px; }
.update-copy { display:flex; align-items:center; flex-wrap:wrap; gap:6px; flex:1; min-width:160px; overflow-wrap:anywhere; }
.update-copy small, .update-hint { color:var(--desk-muted); font-size:12px; }
.update-hint, .update-error { margin-top:8px; }
.update-error { color:#b91c1c; overflow-wrap:anywhere; }
progress { display:block; width:100%; height:4px; margin-top:8px; }
</style>
