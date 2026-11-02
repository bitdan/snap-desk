<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import { t, te } from '@/shared/i18n/i18n'
import { api, desktop } from '@/services/api'
import { onSettingsImported } from '@/features/settings/events'
import { updateSettings } from '@/features/settings/settings-write'
import type { AppUpdateView } from '@/features/settings/types'

const view = ref<AppUpdateView | null>(null)
const preview = ref<boolean | null>(null)
const busy = ref(false), saving = ref(false), error = ref('')
let disposed = false, timer: ReturnType<typeof setInterval> | undefined, refreshing = false
let unsubscribe: (() => void) | undefined
const operating = computed(() => busy.value || ['checking', 'downloading', 'installing'].includes(view.value?.phase ?? ''))
const enabled = computed(() => desktop && view.value?.supported && view.value.configured)
const phaseLabel = computed(() => ({ idle: '尚未检查更新', checking: '正在检查更新…', available: '发现新版本', latest: '当前渠道暂无更新', downloading: '正在下载更新…', ready: '更新已下载并通过校验', installing: '正在退出并安装…', error: '更新失败，可重试' })[view.value?.phase ?? 'idle'])
const progress = computed(() => view.value?.total ? Math.min(100, Math.floor(view.value.downloaded * 100 / view.value.total)) : null)

async function refresh() {
  if (!desktop || refreshing || disposed) return
  refreshing = true
  try {
    const next = await api.appUpdateStatus()
    if (!disposed) view.value = next
  } catch (e) { if (!disposed) error.value = String(e) }
  finally { refreshing = false }
}
async function run(action: () => Promise<AppUpdateView | void>) {
  if (!enabled.value || operating.value || saving.value) return
  busy.value = true; error.value = ''
  try {
    const next = await action()
    if (next && !disposed) view.value = next
  } catch (e) { error.value = String(e) }
  finally { busy.value = false; await refresh() }
}
async function togglePreview() {
  if (preview.value === null || saving.value || operating.value) return
  saving.value = true; error.value = ''
  try {
    const next = !preview.value
    const saved = await updateSettings(current => ({ ...current, receivePrereleaseUpdates: next }))
    preview.value = saved.receivePrereleaseUpdates
    if (view.value) view.value = { ...view.value, availableVersion: null, notes: null, phase: 'idle', error: null }
    // 开关保存成功后刷新候选版本，避免仍展示上一渠道的安装入口。
    if (enabled.value) view.value = await api.checkAppUpdate()
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
async function initialize() {
  if (!desktop) return
  try {
    unsubscribe = await onSettingsImported(settings => { preview.value = settings.receivePrereleaseUpdates; void refresh() })
    if (disposed) { unsubscribe(); return }
    const status = await api.status()
    if (disposed) return
    preview.value = status.settings.receivePrereleaseUpdates
    await refresh()
  } catch (e) { if (!disposed) error.value = String(e) }
}
onMounted(() => {
  void initialize()
  // 页面重新进入也能读取原生端在后台持续的下载，不依赖组件实例保存进度。
  timer = setInterval(() => { void refresh() }, 1000)
})
onUnmounted(() => { disposed = true; if (timer) clearInterval(timer); unsubscribe?.() })
</script>

<template>
  <section class="update-settings">
    <h2 class="section-heading">{{ t('应用更新') }}</h2>
    <div class="setting-row flex-wrap gap-3">
      <span>{{ view ? `v${view.currentVersion}` : t('版本待读取') }}</span>
      <Button variant="outline" :disabled="!enabled || operating || saving" @click="run(api.checkAppUpdate)">{{ t('检查更新') }}</Button>
    </div>
    <div class="setting-row flex-wrap gap-3">
      <label for="receive-prerelease" class="min-w-0 flex-1">{{ t('接收测试版本更新') }}</label>
      <button id="receive-prerelease" type="button" class="update-switch" role="switch" :aria-label="t('接收测试版本更新')" :aria-checked="preview === true" :disabled="!desktop || preview === null || operating || saving" @click="togglePreview"><span aria-hidden="true" /></button>
    </div>
    <p v-if="view?.packageKind === 'unsupported'" class="notice">{{ t('当前安装方式不支持在线更新，请使用 NSIS 安装包重新安装。') }}</p>
    <p v-else-if="view && !view.supported" class="notice">{{ t('在线更新仅支持 Windows x64 发布版本。') }}</p>
    <p v-else-if="view && !view.configured" class="notice">{{ t('此版本未配置更新签名公钥。') }}</p>
    <template v-else-if="view">
      <div class="setting-row flex-wrap gap-3" role="status">
        <span :class="['latest', 'ready'].includes(view.phase) ? 'status-ok' : ''"><Check v-if="['latest', 'ready'].includes(view.phase)" :size="14" />{{ t(phaseLabel) }}<template v-if="view.availableVersion"> · v{{ view.availableVersion }}</template></span>
        <Button v-if="view.availableVersion && ['available', 'error'].includes(view.phase)" :disabled="operating || saving" @click="run(api.downloadAppUpdate)">{{ t('下载更新') }}</Button>
        <Button v-if="view.phase === 'ready'" :disabled="operating || saving" @click="run(api.installAppUpdate)">{{ t('退出并安装') }}</Button>
      </div>
      <div v-if="view.phase === 'downloading'" class="update-progress" role="status"><progress :value="progress ?? undefined" max="100" :aria-label="t('下载进度')" /><span>{{ progress === null ? `${(view.downloaded / 1048576).toFixed(1)} MiB` : `${progress}%` }}</span></div>
      <p v-if="view.phase === 'ready'" class="text-sm text-zinc-500">{{ t('安装会退出应用，请先保存工作并结束录屏。') }}</p>
      <p v-if="view.notes" class="update-notes">{{ view.notes }}</p>
    </template>
    <p v-if="error || view?.error" class="notice error" role="alert">{{ te(error || view?.error) }}</p>
  </section>
</template>

<style scoped>
.update-settings { padding: 0 0 16px; border-bottom: 1px solid var(--desk-border); }
.update-switch { flex-shrink:0; width:38px; height:22px; padding:2px; border-radius:12px; background:#a1a1aa; transition:background .15s; }
.update-switch span { display:block; width:18px; height:18px; border-radius:50%; background:#fff; transition:transform .15s; }
.update-switch[aria-checked=true] { background:#2563eb; }
.update-switch[aria-checked=true] span { transform:translateX(16px); }
.update-switch:disabled { opacity:.45; cursor:not-allowed; }
.update-switch:focus-visible { outline:2px solid #2563eb; outline-offset:3px; }
.update-progress { display: flex; align-items: center; gap: 12px; font-size: 12px; }
.update-progress progress { flex: 1; min-width: 0; height: 8px; }
.update-notes { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 240px; overflow-y: auto; font-size: 12px; }
</style>
