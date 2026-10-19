<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api, desktop } from '@/lib/api'
import { defaultClipboardSettings } from '@/lib/clipboard'
import Button from '@/components/ui/button/Button.vue'

const settings = ref(defaultClipboardSettings())
const ready = ref(false), busy = ref(false), error = ref(''), message = ref('')
async function load() {
  if (!desktop) return
  busy.value = true; error.value = ''
  try { settings.value = (await api.status()).settings.clipboard; ready.value = true }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function save() {
  if (busy.value || !ready.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    // 启停仍由历史页控制；保存前读取最新状态，避免旧表单意外重启记录。
    const current = (await api.status()).settings.clipboard
    await api.clipboardConfigure({ ...current, saveCopiedText: settings.value.saveCopiedText, saveCopiedImages: settings.value.saveCopiedImages, saveCopiedFiles: settings.value.saveCopiedFiles, maxItems: settings.value.maxItems, maxStorageMb: settings.value.maxStorageMb, retainDays: settings.value.retainDays })
    settings.value = (await api.status()).settings.clipboard
    message.value = '剪贴板历史设置已保存。'
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
onMounted(load)
</script>
<template>
  <section class="settings-card clipboard-settings mb-4" aria-labelledby="clipboard-settings-title">
    <h2 id="clipboard-settings-title">剪贴板记录与自动清理</h2>
    <form @submit.prevent="save">
      <fieldset :disabled="busy || !ready || !desktop">
        <div class="setting-row"><label for="history-text">记录文本</label><input id="history-text" v-model="settings.saveCopiedText" type="checkbox" /></div>
        <div class="setting-row"><label for="history-images">记录图片</label><input id="history-images" v-model="settings.saveCopiedImages" type="checkbox" /></div>
        <div class="setting-row"><label for="history-files">记录文件<small>只保存文件／文件夹路径，不备份内容；原文件移动或删除后引用失效。</small></label><input id="history-files" v-model="settings.saveCopiedFiles" type="checkbox" /></div>
        <div class="setting-row"><label for="history-count">最多条目</label><input id="history-count" v-model.number="settings.maxItems" type="number" min="100" max="5000" required /></div>
        <div class="setting-row"><label for="history-size">容量上限（MiB）</label><input id="history-size" v-model.number="settings.maxStorageMb" type="number" min="50" max="2048" required /></div>
        <div class="setting-row"><label for="history-days">保留天数（0 为不限）<small>收藏不会自动删除；启动、保存设置和记录期间自动清理。</small></label><input id="history-days" v-model.number="settings.retainDays" type="number" min="0" max="3650" required /></div>
        <Button type="submit" :disabled="busy || !ready || !desktop">保存历史设置</Button>
      </fieldset>
    </form>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <Button v-if="!ready" variant="ghost" :disabled="busy || !desktop" @click="load">重新读取</Button>
    <p v-if="message" role="status">{{ message }}</p>
  </section>
</template>
<style scoped>
.clipboard-settings { padding:16px; }
h2 { font-size:15px; font-weight:600; }
input[type=number] { width:100px; border:1px solid #d4d4d8; border-radius:6px; padding:6px; }
small { display:block; color:#71717a; }
</style>
