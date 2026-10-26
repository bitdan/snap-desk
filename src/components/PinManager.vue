<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop, onPinsChanged, type PinSummary, type PinManageAction } from '@/lib/api'
import { pinGroups, filterPins, normalizePinGroup } from '@/lib/pin-content'

const pins = ref<PinSummary[]>([])
const busy = ref(false)
const error = ref('')
const groupFilter = ref<string | null>(null)
const groups = computed(() => pinGroups(pins.value))
const visiblePins = computed(() => filterPins(pins.value, groupFilter.value))
const selected = ref<string[]>([])
const groupName = ref('')
watch(groupFilter, () => { selected.value = [] })
let disposed = false
let off: (() => void) | undefined
let revision = 0
/** 窗口增删事件可能紧邻到达，只接受最新查询结果，避免旧列表覆盖新状态。 */
async function refresh() {
  const current = ++revision
  try {
    const result = await api.listPins()
    if (!disposed && current === revision) {
      pins.value = result
      selected.value = selected.value.filter(label => result.some(pin => pin.label === label))
      if (groupFilter.value && !groups.value.includes(groupFilter.value)) groupFilter.value = null
    }
  } catch (e) { if (!disposed && current === revision) error.value = String(e) }
}
async function run(task: () => Promise<unknown>) {
  if (busy.value) return
  busy.value = true; error.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { await refresh(); busy.value = false }
}
function manage(action: PinManageAction, label: string | null = null) { void run(() => api.managePins(action, label, label ? null : groupFilter.value)) }
function assignGroup() {
  void run(async () => {
    await api.setPinGroup([...selected.value], normalizePinGroup(groupName.value))
    selected.value = []
  })
}
onMounted(async () => {
  if (!desktop) return
  try {
    off = await onPinsChanged(() => { void refresh() })
    if (disposed) { off(); return }
    await refresh()
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; off?.() })
</script>

<template>
  <div v-if="error" class="notice error" role="alert">{{ te(error) }}</div>
  <div class="pin-manager-actions">
    <Button :disabled="!desktop || busy" :title='t("直接贴出剪贴板中的图片，快捷键可在设置中修改")' @click="run(api.pinClipboard)">{{ t("剪贴板图片贴图") }}</Button>
    <Button variant="outline" :disabled="!desktop || busy" @click="run(api.pinFile)">{{ t("打开图片贴图") }}</Button>
  </div>
  <div class="pin-manager-actions group-actions">
    <label>{{ t("分组") }} <select v-model="groupFilter" :disabled="busy" :aria-label='t("筛选贴图分组")'><option :value="null">{{ t("全部贴图") }}</option><option value="">{{ t("未分组") }}</option><option v-for="group in groups" :key="group" :value="group">{{ group }}</option></select></label>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('show')">{{ groupFilter === null ? t("全部") : t("本组") }}{{ t("显示") }}</Button>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('hide')">{{ groupFilter === null ? t("全部") : t("本组") }}{{ t("隐藏") }}</Button>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('close')">{{ groupFilter === null ? t("全部") : t("本组") }}{{ t("关闭") }}</Button>
  </div>
  <section class="settings-card" :aria-label='t("当前贴图")'>
    <p v-if="!visiblePins.length" class="pin-empty">{{ pins.length ? t("当前分组没有贴图。") : t("还没有贴图。可从截图工具栏或上方入口创建。") }}</p>
    <div v-for="pin in visiblePins" :key="pin.label" class="setting-row">
      <label class="pin-choice"><input v-model="selected" type="checkbox" :value="pin.label" :disabled="busy" :aria-label="t('选择 {0}', { 0: pin.label })" /><span>{{ pin.label }}<small>{{ pin.width }} × {{ pin.height }} px · {{ pin.visible ? t("显示中") : t("已隐藏") }} · {{ pin.group || t("未分组") }}</small></span></label>
      <div class="pin-manager-actions"><Button variant="outline" :disabled="busy" @click="manage(pin.visible ? 'hide' : 'show', pin.label)">{{ pin.visible ? t("隐藏") : t("显示") }}</Button><Button variant="ghost" :disabled="busy" @click="manage('close', pin.label)">{{ t("关闭") }}</Button></div>
    </div>
  </section>
  <form v-if="pins.length" class="pin-manager-actions group-actions assignment" @submit.prevent="assignGroup">
    <span>{{ t("已选") }} {{ selected.length }} {{ t("张") }}</span>
    <input v-model="groupName" list="pin-groups" :disabled="busy" :placeholder='t("分组名称，留空移出分组")' :aria-label='t("目标分组")' />
    <datalist id="pin-groups"><option v-for="group in groups" :key="group" :value="group" /></datalist>
    <Button type="submit" variant="outline" :disabled="!desktop || busy || !selected.length">{{ t("应用分组") }}</Button>
  </form>
  <p class="scope-note">{{ t("退出会清空贴图，请先保存需要保留的内容。") }}</p>
</template>

<style scoped>
.pin-manager-actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 0 0 16px; }
.setting-row .pin-manager-actions { margin: 0; }
.pin-empty { padding: 24px 0; color: #6e7781; font-size: 13px; }
.group-actions { align-items: center; font-size: 12px; }
.group-actions select, .group-actions input { border: 1px solid #d4d4d8; border-radius: 5px; padding: 7px; background: white; max-width: 220px; }
.pin-choice { display: flex; align-items: center; gap: 10px; min-width: 0; }
.pin-choice small { overflow-wrap: anywhere; }
.assignment { margin-top: 12px; }
</style>
