<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Button from './ui/button/Button.vue'
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
  config: { name: '应用配置目录', description: '保存快捷键、主配置与目录偏好，不是图片或视频输出目录' },
  captures: { name: '截图保存目录', description: '查看上一次记住的保存位置；可在截图设置中切换记住目录' },
  recordings: { name: '录屏保存目录', description: '视频输出位置，恢复默认不会移动已有视频' },
  scrollResults: { name: '长截图结果目录', description: '长截图自动保存的 PNG 结果' },
  clipboard: { name: '剪贴板历史目录', description: '本地历史数据库与图片；请通过历史页清理，勿在运行中手动删除数据库文件' },
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
    <h1>通用设置</h1><p class="subtitle">启动、文件与存储</p>
    <div class="settings-card mb-4">
      <div class="setting-row flex-wrap gap-3">
        <div class="min-w-0 flex-1"><strong>开机自启</strong><small class="block text-zinc-500">登录系统后自动启动到托盘，可直接使用截图快捷键。</small></div>
        <span class="text-sm text-zinc-500" role="status">{{ startupBusy ? '处理中…' : autostart === null ? '状态未读取' : autostart ? '已开启' : '已关闭' }}</span>
        <Button variant="outline" :disabled="!desktop || startupBusy || autostart === null" :aria-pressed="autostart === true" aria-label="开机自启" @click="toggleAutostart">{{ autostart ? '关闭' : '开启' }}</Button>
        <Button variant="ghost" :disabled="!desktop || startupBusy" @click="refreshAutostart">刷新状态</Button>
      </div>
      <p v-if="startupError" class="notice error" role="alert">{{ startupError }}</p>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice" role="status">{{ message }}</p>
    <div class="settings-card">
      <div v-for="directory in directories" :key="directory.kind" class="setting-row flex-wrap gap-3">
        <div class="min-w-0 flex-1"><strong>{{ labels[directory.kind].name }}</strong><small class="block text-zinc-500">{{ labels[directory.kind].description }}</small><p class="mt-2 break-all text-xs">{{ directory.error || directory.path || '尚未记录可用位置，请先保存一张截图。' }}</p></div>
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" :disabled="busy || !directory.path" @click="run(() => api.openAppDirectory(directory.kind))">打开</Button>
          <Button variant="ghost" :disabled="busy || !directory.path" @click="run(() => api.copyAppDirectory(directory.kind), '目录路径已复制。')">复制路径</Button>
          <template v-if="directory.kind === 'recordings'">
            <Button variant="outline" :disabled="busy" @click="run(() => choose())">选择</Button>
            <Button variant="ghost" :disabled="busy" @click="run(() => choose(true))">恢复默认</Button>
          </template>
        </div>
      </div>
      <div class="flex gap-3 py-5"><Button :disabled="!desktop || busy" @click="run(exportConfig)">导出主配置</Button><Button variant="outline" :disabled="!desktop || busy" @click="run(refresh)">刷新目录</Button></div>
    </div>
  </section>
</template>
