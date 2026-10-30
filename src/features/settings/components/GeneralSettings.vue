<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, ref } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import ClipboardSettings from '@/features/clipboard/components/ClipboardSettings.vue'
import ResourceSettings from '@/features/resources/components/ResourceSettings.vue'
import { Check } from 'lucide-vue-next'
import { api, desktop } from '@/services/api'
import type { AppDirectory, DirectoryView } from '@/features/settings/types'
import '@/features/settings/settings.css'

const directories = ref<DirectoryView[]>([])
const busy = ref(false), error = ref(''), message = ref('')
// null 表示尚未读取或读取失败，不能把未知状态误显示为“已关闭”。
const autostart = ref<boolean | null>(null)
const startupBusy = ref(false), startupError = ref('')
async function refreshAutostart() {
  if (!desktop || startupBusy.value) return
  startupBusy.value = true; startupError.value = ''
  try { autostart.value = await api.autostartEnabled() }
  catch (e) { autostart.value = null; startupError.value = String(e) }
  finally { startupBusy.value = false }
}
async function toggleAutostart() {
  if (!desktop || startupBusy.value || autostart.value === null) return
  startupBusy.value = true; startupError.value = ''
  try { autostart.value = await api.setAutostartEnabled(!autostart.value) }
  catch (e) {
    startupError.value = String(e)
    // 写入失败也可能已部分生效，重新查询而不是盲目回滚本地开关。
    try { autostart.value = await api.autostartEnabled() }
    catch { autostart.value = null }
  } finally { startupBusy.value = false }
}
const labels: Record<AppDirectory, { name: string; description: string }> = {
  logs: { name: '应用日志目录', description: '本机日志，分享前请检查内容。' },
  config: { name: '应用配置目录', description: '快捷键与应用设置' },
  data: { name: '应用数据目录', description: '本地数据与恢复文件' },
  captures: { name: '截图保存目录', description: '保存对话框的默认位置' },
  recordings: { name: '录屏保存目录', description: '视频输出位置，恢复默认不会移动已有视频' },
  scrollResults: { name: '滚动截图结果目录', description: '滚动截图自动保存的 PNG 结果' },
  clipboard: { name: '剪贴板历史目录', description: '剪贴板历史，请在应用内清理' },
}
async function refresh() { directories.value = await api.appDirectories() }
async function run(action: () => Promise<unknown>, success = '') {
  if (!desktop || busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await action(); if (success) message.value = success }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function choose(kind: AppDirectory, reset = false) {
  if (kind !== 'captures' && kind !== 'recordings') return
  const result = kind === 'captures'
    ? await api.chooseCaptureDirectory(reset)
    : await api.chooseRecordingDirectory(reset)
  if (result) await refresh()
}
onMounted(() => { void run(refresh); void refreshAutostart() })
</script>

<template>
  <section class="general-settings-page">
    <section class="general-section">
      <slot />
      <div class="setting-row flex-wrap gap-3">
        <div class="min-w-0 flex-1"><strong>{{ t('开机自启') }}</strong></div>
        <span :class="autostart && !startupBusy ? 'status-ok' : 'text-sm text-zinc-500'" role="status"><Check v-if="autostart && !startupBusy" :size="14" />{{ startupBusy ? t('处理中…') : autostart === null ? t('状态未读取') : autostart ? t('已开启') : t('已关闭') }}</span>
        <button type="button" class="autostart-switch" role="switch" :disabled="!desktop || startupBusy || autostart === null" :aria-checked="autostart === true" :aria-label='t("开机自启")' :aria-busy="startupBusy" @click="toggleAutostart"><span aria-hidden="true" /></button>
        <Button v-if="autostart === null && !startupBusy" variant="ghost" :disabled="!desktop" @click="refreshAutostart">{{ t('重新读取') }}</Button>
      </div>
      <p v-if="startupError" class="notice error" role="alert">{{ te(startupError) }}</p>
    </section>
    <ClipboardSettings :disabled="busy" />
    <section class="general-section general-font"><h2>{{ t('导出字体') }}</h2><ResourceSettings kind="font" /></section>
    <section class="general-section general-storage">
    <h2>{{ t('文件与存储') }}</h2>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
    <div class="settings-card">
      <div v-for="directory in directories.filter(item => item.kind !== 'logs')" :key="directory.kind" class="setting-row directory-row">
        <div class="directory-content"><strong>{{ t(labels[directory.kind].name) }}</strong><p class="directory-path" :class="{ 'directory-error': directory.error }">{{ te(directory.error) || directory.path || t('系统默认位置') }}</p><small v-if="directory.kind === 'captures'">{{ t('作为保存对话框的默认位置，保存时仍可另选目录。') }}</small><small v-if="directory.kind === 'clipboard'">{{ t('请通过剪贴板页面清理历史，勿手动删除数据库。') }}</small></div>
        <div class="directory-actions">
          <Button variant="outline" :disabled="busy || !directory.path" @click="run(() => api.openAppDirectory(directory.kind))">{{ t('打开') }}</Button>
          <Button variant="ghost" :disabled="busy || !directory.path" @click="run(() => api.copyAppDirectory(directory.kind), '目录路径已复制。')">{{ t('复制路径') }}</Button>
          <template v-if="directory.kind === 'captures' || directory.kind === 'recordings'">
            <Button variant="outline" :disabled="busy" @click="run(() => choose(directory.kind))">{{ t('选择') }}</Button>
            <Button variant="ghost" :disabled="busy" @click="run(() => choose(directory.kind, true))">{{ t('恢复默认') }}</Button>
          </template>
        </div>
      </div>
      <Button v-if="error && !directories.length" variant="ghost" :disabled="!desktop || busy" @click="run(refresh)">{{ t('重新读取') }}</Button>
    </div>
    </section>
  </section>
</template>

<style scoped>
.autostart-switch { flex-shrink:0; width:38px; height:22px; padding:2px; border-radius:12px; background:#a1a1aa; transition:background .15s; }
.autostart-switch span { display:block; width:18px; height:18px; border-radius:50%; background:#fff; transition:transform .15s; }
.autostart-switch[aria-checked=true] { background:#2563eb; }
.autostart-switch[aria-checked=true] span { transform:translateX(16px); }
.autostart-switch:disabled { opacity:.45; cursor:not-allowed; }
.autostart-switch:focus-visible { outline:2px solid #2563eb; outline-offset:3px; }
</style>
