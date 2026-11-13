<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'
import { t, te } from '@/shared/i18n/i18n'
import { api, desktop } from '@/services/api'
import { onSettingsImported, requestUpdateWindow } from '@/features/settings/events'
import { updateSettings } from '@/features/settings/settings-write'
import type { AppUpdateView } from '@/features/settings/types'
import { updatePreview } from '@/features/settings/update-preview'

const view = ref<AppUpdateView | null>(null)
const preview = ref<boolean | null>(null)
const busy = ref(false), saving = ref(false), error = ref('')
let disposed = false, timer: ReturnType<typeof setInterval> | undefined, refreshing = false
let unsubscribe: (() => void) | undefined
const operating = computed(() => busy.value || ['checking', 'downloading', 'installing'].includes(view.value?.phase ?? ''))
const enabled = computed(() => (desktop || updatePreview) && view.value?.supported && view.value.configured)
const phaseLabel = computed(() => ({ idle: '尚未检查更新', checking: '正在检查更新…', available: '发现新版本', latest: '当前渠道暂无更新', downloading: '正在下载更新…', ready: '更新已下载并通过校验', installing: '正在退出并安装…', error: '更新失败，可重试' })[view.value?.phase ?? 'idle'])
const hasUpdate = computed(() => !!view.value?.availableVersion)
// 测试版构建由原生端固定接收测试版；开关显示为开启且不可改，避免界面与实际渠道不一致。
const prereleaseBuild = computed(() => !!view.value?.currentVersion.includes('-'))

async function refresh() {
  if ((!desktop && !updatePreview) || refreshing || disposed) return
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
  if (prereleaseBuild.value || preview.value === null || saving.value || operating.value) return
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
  if (updatePreview) { await refresh(); return }
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
    <p v-if="updatePreview" class="notice">{{ t('模拟更新，不会下载文件或退出应用。') }}</p>
    <div class="settings-card">
    <SettingRow :label="t('测试版更新')" for="receive-prerelease" :tip="prereleaseBuild ? t('当前为测试版，始终接收测试版更新；升级到正式版后按此开关决定。') : undefined">
      <SwitchControl id="receive-prerelease" :model-value="prereleaseBuild || preview === true" :disabled="prereleaseBuild || updatePreview || !desktop || preview === null || operating || saving" @update:model-value="togglePreview" />
    </SettingRow>
    <div class="setting-row flex-wrap gap-3">
      <div class="update-version"><span>{{ view ? `v${view.currentVersion}` : t('版本待读取') }}</span><span v-if="view && !hasUpdate && ['checking', 'latest', 'error'].includes(view.phase)" role="status" :class="view.phase === 'latest' ? 'status-ok' : ''"><Check v-if="view.phase === 'latest'" :size="14" />{{ t(phaseLabel) }}</span></div>
      <Button variant="outline" :disabled="!enabled || (!hasUpdate && operating) || saving" @click="hasUpdate ? requestUpdateWindow() : run(api.checkAppUpdate)"><span v-if="hasUpdate" class="update-dot" aria-hidden="true" />{{ t(hasUpdate ? '有更新' : '检查更新') }}</Button>
    </div>
    </div>
    <p v-if="view?.packageKind === 'unsupported'" class="notice">{{ t('当前安装方式不支持在线更新，请使用 NSIS 安装包重新安装。') }}</p>
    <p v-else-if="view?.supported && !view.configured" class="notice">{{ t('此版本未配置更新签名公钥。') }}</p>
    <p v-if="error || view?.error" class="notice error" role="alert">{{ te(error || view?.error) }}</p>
  </section>
</template>

<style scoped>
.update-version { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; }
.update-dot { width:6px; height:6px; flex:none; border-radius:50%; background:#dc2626; }
</style>
