<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'
import SegmentedControl from '@/shared/ui/SegmentedControl.vue'
import { t, te } from '@/shared/i18n/i18n'
import { api, desktop } from '@/services/api'
import { onSettingsImported, requestUpdateWindow } from '@/features/settings/events'
import { updateSettings } from '@/features/settings/settings-write'
import { updateCheckIntervals, validateUpdateMirror, validateUpdateProxy, type UpdateNetwork } from '@/features/settings/settings'
import type { AppUpdateView } from '@/features/settings/types'
import { updatePreview } from '@/features/settings/update-preview'

const view = ref<AppUpdateView | null>(null)
const preview = ref<boolean | null>(null)
// 后台复查间隔（分钟）；读取到设置前为 null，控件保持禁用。
const interval = ref<number | null>(null)
// 下载镜像与代理：输入框编辑草稿，失焦或回车时校验并保存；读取到设置前为 null。
const network = ref<UpdateNetwork | null>(null)
const mirrorDraft = ref(''), proxyDraft = ref('')
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
/** 只保存间隔；原生端每分钟重读一次，不需要重启或立即检查。失败时保留已生效的档位。 */
async function chooseInterval(minutes: number) {
  if (interval.value === null || saving.value) return
  saving.value = true; error.value = ''
  try {
    const saved = await updateSettings(current => ({ ...current, updateCheckMinutes: minutes }))
    interval.value = saved.updateCheckMinutes
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
function applyNetwork(value: UpdateNetwork) {
  network.value = { ...value }; mirrorDraft.value = value.mirror; proxyDraft.value = value.proxy
}
/** 输入只改草稿；按输入框 id 区分镜像与代理。 */
function draftInput(event: Event) {
  const input = event.target as HTMLInputElement
  if (input.id === 'update-mirror') mirrorDraft.value = input.value
  else proxyDraft.value = input.value
}
/** 去掉首尾空白后保存；与已保存值相同时不写设置，校验失败时保留草稿并提示。 */
async function saveNetwork() {
  if (!network.value || saving.value) return
  const next = { mirror: mirrorDraft.value.trim(), proxy: proxyDraft.value.trim() }
  mirrorDraft.value = next.mirror; proxyDraft.value = next.proxy
  if (next.mirror === network.value.mirror && next.proxy === network.value.proxy) return
  const invalid = validateUpdateMirror(next.mirror) ?? validateUpdateProxy(next.proxy)
  if (invalid) { error.value = invalid; return }
  saving.value = true; error.value = ''
  try {
    const saved = await updateSettings(current => ({ ...current, updateNetwork: next }))
    applyNetwork(saved.updateNetwork)
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
async function initialize() {
  if (updatePreview) { await refresh(); return }
  if (!desktop) return
  try {
    unsubscribe = await onSettingsImported(settings => { preview.value = settings.receivePrereleaseUpdates; interval.value = settings.updateCheckMinutes; applyNetwork(settings.updateNetwork); void refresh() })
    if (disposed) { unsubscribe(); return }
    const status = await api.status()
    if (disposed) return
    preview.value = status.settings.receivePrereleaseUpdates
    interval.value = status.settings.updateCheckMinutes
    applyNetwork(status.settings.updateNetwork)
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
    <SettingRow :label="t('检查更新间隔')">
      <SegmentedControl :model-value="interval ?? 60" :label="t('检查更新间隔')" :disabled="updatePreview || !desktop || interval === null || saving" :options="updateCheckIntervals.map(item => ({ value: item.value, label: t(item.label) }))" @update:model-value="chooseInterval" />
    </SettingRow>
    <SettingRow :label="t('下载镜像')" for="update-mirror" :tip="t('无法直连 GitHub 时填写镜像前缀，下载时放在 github.com 地址前，失败会改为直连。安装包仍须通过签名校验，镜像无法替换内容。')">
      <input id="update-mirror" :value="mirrorDraft" class="update-input" type="url" autocomplete="off" spellcheck="false" maxlength="512" :placeholder="t('可选，如 https://mirror.example/')" :disabled="updatePreview || !desktop || network === null || saving" @input="draftInput" @change="saveNetwork" @keydown.enter.prevent="saveNetwork" />
    </SettingRow>
    <SettingRow :label="t('更新代理')" for="update-proxy" :tip="t('只用于检查和下载更新；留空时使用系统代理。支持 http、https、socks5。')">
      <input id="update-proxy" :value="proxyDraft" class="update-input" type="text" autocomplete="off" spellcheck="false" maxlength="512" :placeholder="t('可选，如 http://127.0.0.1:7890')" :disabled="updatePreview || !desktop || network === null || saving" @input="draftInput" @change="saveNetwork" @keydown.enter.prevent="saveNetwork" />
    </SettingRow>
    <div class="setting-row">
      <div class="update-version"><span>{{ view ? `v${view.currentVersion}` : t('版本待读取') }}</span><span v-if="view && !hasUpdate && ['checking', 'latest', 'error'].includes(view.phase)" role="status" :class="view.phase === 'latest' ? 'status-ok' : 'text-xs text-zinc-500'"><Check v-if="view.phase === 'latest'" :size="14" />{{ t(phaseLabel) }}</span></div>
      <Button variant="outline" :disabled="!enabled || (!hasUpdate && operating) || saving" @click="hasUpdate ? requestUpdateWindow() : run(api.checkAppUpdate)"><span v-if="hasUpdate" class="update-dot" aria-hidden="true" />{{ t(hasUpdate ? '有更新' : '检查更新') }}</Button>
    </div>
    </div>
    <p v-if="view?.packageKind === 'unsupported'" class="notice">{{ t('当前安装方式不支持在线更新，请从发布页下载新版本。') }}</p>
    <p v-else-if="view?.supported && !view.configured" class="notice">{{ t('此版本未配置更新签名公钥。') }}</p>
    <p v-if="error || view?.error" class="notice error" role="alert">{{ te(error || view?.error) }}</p>
  </section>
</template>

<style scoped>
.update-version { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; }
.update-dot { width:6px; height:6px; flex:none; border-radius:50%; background:#dc2626; }
.update-input { width:15rem; max-width:100%; }
</style>
