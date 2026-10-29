<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, ref } from 'vue'
import { Check } from 'lucide-vue-next'
import { api, desktop } from '@/services/api'
import { defaultClipboardSettings } from '@/features/clipboard/clipboard'
import Button from '@/shared/ui/button/Button.vue'

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
  <section class="general-section clipboard-settings" aria-labelledby="clipboard-settings-title">
    <h2 id="clipboard-settings-title">{{ t("剪贴板记录与自动清理") }}</h2>
    <form @submit.prevent="save">
      <fieldset :disabled="busy || !ready || !desktop">
        <div class="clipboard-record-options">
          <label for="history-text"><input id="history-text" v-model="settings.saveCopiedText" type="checkbox" />{{ t("记录文本") }}</label>
          <label for="history-images"><input id="history-images" v-model="settings.saveCopiedImages" type="checkbox" />{{ t("记录图片") }}</label>
          <label for="history-files"><input id="history-files" v-model="settings.saveCopiedFiles" type="checkbox" />{{ t("记录文件") }}</label>
        </div>
        <div class="clipboard-limits">
          <div class="setting-field"><label for="history-count">{{ t("最多条目") }}</label><input id="history-count" v-model.number="settings.maxItems" type="number" min="100" max="5000" required /></div>
          <div class="setting-field"><label for="history-size">{{ t("容量上限（MiB）") }}</label><input id="history-size" v-model.number="settings.maxStorageMb" type="number" min="50" max="2048" required /></div>
          <div class="setting-field"><label for="history-days">{{ t("保留天数（0 为不限）") }}</label><input id="history-days" v-model.number="settings.retainDays" type="number" min="0" max="3650" required /></div>
        </div>
        <div class="clipboard-settings-footer"><p class="settings-note">{{ t("仅记录路径，不备份文件。") }} {{ t("自动清理不删除收藏。") }}</p><Button type="submit" variant="outline" :disabled="busy || !ready || !desktop">{{ t("保存历史设置") }}</Button></div>
      </fieldset>
    </form>
    <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    <Button v-if="!ready" variant="ghost" :disabled="busy || !desktop" @click="load">{{ t("重新读取") }}</Button>
    <p v-if="message" class="status-ok" role="status"><Check :size="14" />{{ t(message) }}</p>
  </section>
</template>
