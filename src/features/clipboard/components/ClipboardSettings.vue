<script setup lang="ts">
import { Check, ClipboardList } from 'lucide-vue-next'
import { t, te } from '@/shared/i18n/i18n'
import { onMounted, onUnmounted, ref } from 'vue'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { api, desktop, onClipboardState } from '@/services/api'
import { defaultClipboardSettings, type ClipboardStatus } from '@/features/clipboard/clipboard'
import Button from '@/shared/ui/button/Button.vue'

/** 主窗口的剪贴板页：唤出独立窗口，并集中放置记录开关、记录范围与自动清理。 */
defineProps<{ disabled?: boolean; shortcut?: string }>()
const settings = ref(defaultClipboardSettings())
const ready = ref(false), busy = ref(false), error = ref('')
// null 表示状态尚未读取，不能把未知状态显示成“已暂停”。
const status = ref<ClipboardStatus | null>(null)
let disposed = false, offState: (() => void) | undefined
async function load() {
  if (!desktop) return
  busy.value = true; error.value = ''
  try {
    const [app, state] = await Promise.all([api.status(), api.clipboardStatus()])
    settings.value = app.settings.clipboard; status.value = state; ready.value = true
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function open() {
  error.value = ''
  try { await api.clipboardWindow('open') } catch (e) { error.value = String(e) }
}
/** 启停单独提交：后端基于最新配置只改这一项，不会带上尚未保存的限额草稿。 */
async function toggleRecording() {
  if (!status.value || busy.value) return
  busy.value = true; error.value = ''
  try {
    status.value = await api.clipboardSetEnabled(!status.value.enabled)
    settings.value.enabled = status.value.enabled
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
const autosave = useAutoSave(async () => {
  if (!ready.value || !desktop) return
  const value = { ...settings.value }
  // 只覆盖当前面板字段，记录启停沿用后端最新值，避免与剪贴板窗口里的开关互相覆盖。
  const current = (await api.status()).settings.clipboard
  await api.clipboardConfigure({ ...current, saveCopiedText: value.saveCopiedText, saveCopiedImages: value.saveCopiedImages, saveCopiedFiles: value.saveCopiedFiles, maxItems: value.maxItems, maxStorageMb: value.maxStorageMb, retainDays: value.retainDays })
}, () => {
  const value = settings.value
  return !Number.isInteger(value.maxItems) || value.maxItems < 100 || value.maxItems > 5000
    || !Number.isInteger(value.maxStorageMb) || value.maxStorageMb < 50 || value.maxStorageMb > 2048
    || !Number.isInteger(value.retainDays) || value.retainDays < 0 || value.retainDays > 3650
    ? '历史限制无效：条目 100～5000、容量 50～2048 MiB、保留期 0～3650 天。' : null
})
async function flush() { await autosave.flush(); return ready.value && !error.value && !autosave.error.value }
defineExpose({ flush, load })
onMounted(async () => {
  void load()
  if (!desktop) return
  // 剪贴板窗口里继续/暂停记录后，这里的开关同步跟随。
  try { const off = await onClipboardState(value => { status.value = value }); if (disposed) off(); else offState = off }
  catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; offState?.() })
</script>
<template>
  <section class="general-section clipboard-settings" aria-labelledby="clipboard-settings-title">
    <div class="clipboard-launch">
      <Button :disabled="!desktop" @click="open"><ClipboardList :size="16" aria-hidden="true" />{{ t("打开剪贴板") }}</Button>
      <kbd v-if="shortcut">{{ shortcut }}</kbd>
      <span v-if="status" class="settings-note">{{ status.count }} {{ t("条 ·") }} {{ (status.byteSize / 1048576).toFixed(1) }} MiB</span>
    </div>
    <div class="setting-row flex-wrap gap-3">
      <div class="min-w-0 flex-1"><strong>{{ t("记录剪贴板历史") }}</strong></div>
      <span :class="status?.enabled ? 'status-ok' : 'text-sm text-zinc-500'" role="status"><Check v-if="status?.enabled" :size="14" />{{ !status ? t("状态未读取") : !status.supported ? t("当前平台不支持") : status.enabled ? t("正在记录") : t("已暂停") }}</span>
      <button type="button" class="settings-switch" role="switch" :disabled="disabled || !desktop || busy || !status?.supported" :aria-checked="status?.enabled === true" :aria-label='t("记录剪贴板历史")' :aria-busy="busy" @click="toggleRecording"><span aria-hidden="true" /></button>
    </div>
    <p v-if="status?.error" class="notice error" role="alert">{{ te(status.error) }}</p>
    <h2 id="clipboard-settings-title">{{ t("剪贴板记录与自动清理") }}</h2>
    <form @submit.prevent @input="autosave.schedule" @change="autosave.commit" @focusout="autosave.flush">
      <fieldset :disabled="disabled || busy || !ready || !desktop">
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
        <div class="clipboard-settings-footer"><p class="settings-note">{{ t("自动清理不删除收藏。") }}</p><span class="settings-save-status" role="status" v-if="autosave.saving.value || autosave.saved.value" :class="{ 'status-ok': !autosave.saving.value }"><Check v-if="!autosave.saving.value" :size="14" class="status-check" />{{ autosave.saving.value ? t("正在保存…") : t("已自动保存") }}</span></div>
      </fieldset>
    </form>
    <p v-if="error || autosave.error.value" class="notice error" role="alert">{{ te(error || autosave.error.value) }}<Button v-if="ready" variant="ghost" @click="autosave.retry">{{ t("重试") }}</Button></p>
    <Button v-if="!ready" variant="ghost" :disabled="busy || !desktop" @click="load">{{ t("重新读取") }}</Button>
  </section>
</template>
