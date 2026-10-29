<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, ref } from 'vue'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { api, desktop } from '@/services/api'
import { defaultClipboardSettings } from '@/features/clipboard/clipboard'
import Button from '@/shared/ui/button/Button.vue'

const settings = ref(defaultClipboardSettings())
const ready = ref(false), busy = ref(false), error = ref('')
async function load() {
  if (!desktop) return
  busy.value = true; error.value = ''
  try { settings.value = (await api.status()).settings.clipboard; ready.value = true }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
const autosave = useAutoSave(async () => {
  if (!ready.value || !desktop) return
  const value = { ...settings.value }
  // 只覆盖当前面板字段，保留历史页最新的记录启停状态。
  const current = (await api.status()).settings.clipboard
  await api.clipboardConfigure({ ...current, saveCopiedText: value.saveCopiedText, saveCopiedImages: value.saveCopiedImages, saveCopiedFiles: value.saveCopiedFiles, maxItems: value.maxItems, maxStorageMb: value.maxStorageMb, retainDays: value.retainDays })
}, () => {
  const value = settings.value
  return !Number.isInteger(value.maxItems) || value.maxItems < 100 || value.maxItems > 5000
    || !Number.isInteger(value.maxStorageMb) || value.maxStorageMb < 50 || value.maxStorageMb > 2048
    || !Number.isInteger(value.retainDays) || value.retainDays < 0 || value.retainDays > 3650
    ? '历史限制无效：条目 100～5000、容量 50～2048 MiB、保留期 0～3650 天。' : null
})
onMounted(load)
</script>
<template>
  <section class="general-section clipboard-settings" aria-labelledby="clipboard-settings-title">
    <h2 id="clipboard-settings-title">{{ t("剪贴板记录与自动清理") }}</h2>
    <form @submit.prevent @input="autosave.schedule" @change="autosave.commit" @focusout="autosave.flush">
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
        <div class="clipboard-settings-footer"><p class="settings-note">{{ t("仅记录路径，不备份文件。") }} {{ t("自动清理不删除收藏。") }}</p><span class="settings-save-status" role="status">{{ autosave.saving.value ? t("正在保存…") : autosave.saved.value ? t("已自动保存") : t("更改自动保存") }}</span></div>
      </fieldset>
    </form>
    <p v-if="error || autosave.error.value" class="notice error" role="alert">{{ te(error || autosave.error.value) }}<Button v-if="ready" variant="ghost" @click="autosave.retry">{{ t("重试") }}</Button></p>
    <Button v-if="!ready" variant="ghost" :disabled="busy || !desktop" @click="load">{{ t("重新读取") }}</Button>
  </section>
</template>
