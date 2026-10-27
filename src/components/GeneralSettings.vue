<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { onMounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
import ClipboardSettings from './clipboard/ClipboardSettings.vue'
import ResourceSettings from './ResourceSettings.vue'
import { Check } from 'lucide-vue-next'
import { api, desktop, type AppDirectory, type DirectoryView } from '@/lib/api'

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
  captures: { name: '截图保存目录', description: '上次保存位置' },
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
async function choose(reset = false) {
  await api.chooseRecordingDirectory(reset)
  await refresh()
}
async function exportConfig() {
  if (await api.exportSettings()) message.value = '主配置已导出，不包含图片、视频或目录偏好文件。'
}
onMounted(() => { void run(refresh); void refreshAutostart() })
</script>

<template>
  <section>
    <div class="settings-card mb-4">
      <div class="setting-row flex-wrap gap-3">
        <div class="min-w-0 flex-1"><strong>{{ t('开机自启') }}</strong></div>
        <span :class="autostart && !startupBusy ? 'status-ok' : 'text-sm text-zinc-500'" role="status"><Check v-if="autostart && !startupBusy" :size="14" />{{ startupBusy ? t('处理中…') : autostart === null ? t('状态未读取') : autostart ? t('已开启') : t('已关闭') }}</span>
        <Button variant="outline" :disabled="!desktop || startupBusy || autostart === null" :aria-pressed="autostart === true" :aria-label='t("开机自启")' @click="toggleAutostart">{{ autostart ? t('关闭自启') : t('开启') }}</Button>
        <Button variant="ghost" :disabled="!desktop || startupBusy" @click="refreshAutostart">{{ t('刷新状态') }}</Button>
      </div>
      <p v-if="startupError" class="notice error" role="alert">{{ te(startupError) }}</p>
    </div>
    <ClipboardSettings />
    <ResourceSettings kind="font" />
    <h2 class="section-heading">{{ t('文件与存储') }}</h2>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    <p v-if="message" class="notice" role="status">{{ t(message) }}</p>
    <div class="settings-card">
      <div v-for="directory in directories.filter(item => item.kind !== 'logs')" :key="directory.kind" class="setting-row flex-wrap gap-3">
        <div class="min-w-0 flex-1"><strong>{{ t(labels[directory.kind].name) }}</strong><small v-if="directory.kind === 'clipboard'" class="block text-zinc-500">{{ t('请通过剪贴板页面清理历史，勿手动删除数据库。') }}</small><p class="mt-2 break-all text-xs">{{ te(directory.error) || directory.path || t('尚未记录可用位置，请先保存一张截图。') }}</p></div>
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" :disabled="busy || !directory.path" @click="run(() => api.openAppDirectory(directory.kind))">{{ t('打开') }}</Button>
          <Button variant="ghost" :disabled="busy || !directory.path" @click="run(() => api.copyAppDirectory(directory.kind), '目录路径已复制。')">{{ t('复制路径') }}</Button>
          <template v-if="directory.kind === 'recordings'">
            <Button variant="outline" :disabled="busy" @click="run(() => choose())">{{ t('选择') }}</Button>
            <Button variant="ghost" :disabled="busy" @click="run(() => choose(true))">{{ t('恢复默认') }}</Button>
          </template>
        </div>
      </div>
      <div class="flex gap-2 py-3"><Button :disabled="!desktop || busy" @click="run(exportConfig)">{{ t('导出主配置') }}</Button><Button variant="outline" :disabled="!desktop || busy" @click="run(refresh)">{{ t('刷新目录') }}</Button></div>
    </div>
  </section>
</template>
