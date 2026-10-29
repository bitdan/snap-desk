<script setup lang="ts">
import { Pin, ImagePlus, Clipboard, Eye, EyeOff, X, Check } from 'lucide-vue-next'
import PinAppearanceSettings from './PinAppearanceSettings.vue'
import { defaultPinAppearance, pinAppearanceStyle } from '../pins'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop, onPinsChanged } from '@/services/api'
import type { PinSummary, PinManageAction } from '@/features/pins/types'
import { pinGroups, filterPins, normalizePinGroup } from '@/features/pins/pin-content'

const pins = ref<PinSummary[]>([])
const busy = ref(false)
const error = ref('')
const ready = ref(false)
const appearance = ref(defaultPinAppearance())
const visibility = ref('all')
const groupFilter = ref<string | null>(null)
const groups = computed(() => pinGroups(pins.value))
const groupedPins = computed(() => filterPins(pins.value, groupFilter.value))
const visiblePins = computed(() => groupedPins.value.filter(pin => visibility.value === 'all' || pin.visible === (visibility.value === 'visible')))
const allSelected = computed(() => visiblePins.value.length > 0 && selected.value.length === visiblePins.value.length)
const selected = ref<string[]>([])
const groupName = ref('')
watch([groupFilter, visibility], () => { selected.value = [] })
let disposed = false
let off: (() => void) | undefined
let revision = 0
/** 窗口增删事件可能紧邻到达，只接受最新查询结果，避免旧列表覆盖新状态。 */
async function refresh() {
  const current = ++revision
  try {
    const result = await api.listPins()
    if (!disposed && current === revision) {
      if (!ready.value) error.value = ''
      pins.value = result; ready.value = true
      selected.value = selected.value.filter(label => visiblePins.value.some(pin => pin.label === label))
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
function manage(action: PinManageAction, label: string) { void run(() => api.managePins(action, label, null)) }
function manageSelected(action: PinManageAction) {
  const labels = [...selected.value]
  void run(async () => { for (const label of labels) await api.managePins(action, label, null) })
}
function toggleAll() { selected.value = allSelected.value ? [] : visiblePins.value.map(pin => pin.label) }
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
  <section class="pin-manager">
    <div class="pin-toolbar">
      <div class="pin-actions"><Button :disabled="!desktop || busy" @click="run(api.pinClipboard)"><Clipboard :size="15" />{{ t('剪贴板贴图') }}</Button><Button variant="outline" :disabled="!desktop || busy" @click="run(api.pinFile)"><ImagePlus :size="15" />{{ t('打开图片贴图') }}</Button></div>
      <span v-if="ready" class="pin-count">{{ t('共 {0} 张贴图', { 0: pins.length }) }}</span>
    </div>
    <div class="pin-filters">
      <div class="pin-tabs" role="group" :aria-label="t('筛选贴图状态')"><button v-for="item in [{ value: 'all', label: t('全部') }, { value: 'visible', label: t('显示中') }, { value: 'hidden', label: t('已隐藏') }]" :key="item.value" :aria-pressed="visibility === item.value" :disabled="busy" @click="visibility = item.value">{{ item.label }}</button></div>
      <SelectControl v-model="groupFilter" :disabled="busy" :aria-label="t('筛选贴图分组')" :options="[{ value: null, label: t('全部贴图') }, { value: '', label: t('未分组') }, ...groups.map(value => ({ value, label: value }))]" />
    </div>
    <div v-if="error" class="notice error" role="alert">{{ te(error) }}<Button variant="ghost" :disabled="busy" @click="refresh">{{ t('重试') }}</Button></div>
    <p v-if="!ready && !error" class="pin-empty" role="status">{{ desktop ? t('正在读取贴图…') : t('请在桌面应用中使用贴图。') }}</p>
    <template v-else>
      <div v-if="visiblePins.length" class="pin-selection-row"><label><input type="checkbox" :checked="allSelected" :indeterminate="selected.length > 0 && !allSelected" :disabled="busy" @change="toggleAll" />{{ t('选择全部') }}</label><span>{{ t('当前 {0} 张', { 0: visiblePins.length }) }}</span></div>
      <div v-if="selected.length" class="pin-batch" role="group" :aria-label="t('批量操作')"><strong>{{ t('已选 {0} 张', { 0: selected.length }) }}</strong><Button variant="ghost" :disabled="busy" @click="manageSelected('show')"><Eye :size="14" />{{ t('显示') }}</Button><Button variant="ghost" :disabled="busy" @click="manageSelected('hide')"><EyeOff :size="14" />{{ t('隐藏') }}</Button><Button variant="ghost" :disabled="busy" @click="manageSelected('close')"><X :size="14" />{{ t('关闭') }}</Button><Button variant="ghost" :disabled="busy" @click="selected = []">{{ t('取消选择') }}</Button></div>
      <section class="pin-list" :aria-label="t('当前贴图')">
        <div v-if="!visiblePins.length" class="pin-empty"><Pin :size="24" /><p>{{ pins.length ? t('没有符合筛选条件的贴图。') : t('还没有贴图。可从截图工具栏或上方入口创建。') }}</p><Button v-if="pins.length" variant="ghost" @click="visibility = 'all'; groupFilter = null">{{ t('清除筛选') }}</Button></div>
        <article v-for="pin in visiblePins" :key="pin.label" class="pin-entry" :class="{ selected: selected.includes(pin.label) }">
          <input :id="`pin-choice-${pin.label}`" v-model="selected" type="checkbox" :value="pin.label" :disabled="busy" :aria-label="t('选择 {0}', { 0: pin.label })" />
          <label class="pin-thumbnail" :style="pinAppearanceStyle(appearance, 80, 56)" :for="`pin-choice-${pin.label}`"><img v-if="pin.thumbnail" :src="pin.thumbnail" :alt="t('贴图缩略图')" loading="lazy" /><Pin v-else :size="22" /></label>
          <div class="pin-description"><label :for="`pin-choice-${pin.label}`" class="pin-name">{{ pin.label }}</label><span class="pin-metadata">{{ pin.width }} × {{ pin.height }} px <span aria-hidden="true">·</span> {{ pin.group || t('未分组') }}</span><span class="pin-visibility" :class="{ visible: pin.visible }"><Check v-if="pin.visible" :size="12" /><EyeOff v-else :size="12" />{{ pin.visible ? t('显示中') : t('已隐藏') }}</span></div>
          <div class="pin-entry-actions"><Button variant="outline" :disabled="busy" @click="manage(pin.visible ? 'hide' : 'show', pin.label)"><EyeOff v-if="pin.visible" :size="14" /><Eye v-else :size="14" />{{ pin.visible ? t('隐藏') : t('显示') }}</Button><Button variant="ghost" :disabled="busy" :aria-label="t('关闭 {0}', { 0: pin.label })" :title="t('关闭贴图')" @click="manage('close', pin.label)"><X :size="15" /></Button></div>
        </article>
      </section>
      <form v-if="selected.length" class="pin-group-assignment" @submit.prevent="assignGroup"><label for="pin-group-name">{{ t('移动到分组') }}</label><input id="pin-group-name" v-model="groupName" list="pin-groups" :disabled="busy" :placeholder="t('分组名称，留空移出分组')" /><datalist id="pin-groups"><option v-for="group in groups" :key="group" :value="group" /></datalist><Button type="submit" variant="outline" :disabled="!desktop || busy">{{ t('应用分组') }}</Button></form>
    </template>
    <PinAppearanceSettings @preview="appearance = $event" />
    <p class="pin-session-note">{{ t('退出会清空贴图，请先保存需要保留的内容。') }}</p>
  </section>
</template>

<style scoped>
.pin-toolbar, .pin-actions, .pin-filters, .pin-entry-actions, .pin-selection-row, .pin-selection-row label { display:flex; align-items:center; flex-wrap:wrap; gap:8px; }
.pin-toolbar { justify-content:space-between; padding-bottom:16px; border-bottom:1px solid var(--desk-border); }
.pin-count, .pin-selection-row, .pin-session-note { color:var(--desk-muted); font-size:12px; }
.pin-filters { justify-content:space-between; gap:12px; margin:14px 0; }
.pin-tabs { display:flex; gap:4px; }
.pin-tabs button { padding:6px 12px; min-height:32px; border-radius:6px; color:var(--desk-muted); }
.pin-tabs button[aria-pressed=true] { background:var(--desk-selected); color:var(--desk-accent); font-weight:600; }
.pin-tabs button:hover { background:var(--desk-soft); }
.pin-selection-row { justify-content:space-between; padding:8px 12px; }
.pin-selection-row label { gap:10px; cursor:pointer; }
.pin-entry { display:flex; align-items:center; gap:16px; min-height:96px; padding:14px 12px; border-bottom:1px solid var(--desk-border); border-radius:6px; }
.pin-entry:hover { background:var(--desk-soft); }
.pin-entry.selected { background:var(--desk-selected); }
.pin-thumbnail { display:grid; place-items:center; width:80px; height:56px; flex:none; overflow:hidden; border:1px solid var(--desk-border); background:var(--desk-bg); color:var(--desk-muted); cursor:pointer; }
.pin-thumbnail img { width:100%; height:100%; object-fit:contain; }
.pin-description { display:grid; gap:3px; flex:1; min-width:0; }
.pin-name { font-weight:600; cursor:pointer; overflow-wrap:anywhere; }
.pin-metadata { display:flex; flex-wrap:wrap; gap:6px; color:var(--desk-muted); font-size:12px; }
.pin-visibility { display:inline-flex; align-items:center; gap:4px; font-size:11px; color:var(--desk-muted); }
.pin-visibility.visible { color:var(--ui-success-fg); }
.pin-entry-actions { flex:none; }
.pin-batch { display:flex; flex-wrap:wrap; align-items:center; gap:4px; padding:8px 12px; background:var(--desk-selected); border-radius:6px; margin-bottom:4px; }
.pin-batch strong { font-size:12px; margin-right:auto; color:var(--desk-accent); }
.app-shell .pin-empty { display:flex; flex-direction:column; gap:12px; min-height:160px; }
.pin-group-assignment { display:flex; align-items:center; flex-wrap:wrap; gap:8px; margin-top:12px; font-size:12px; }
.pin-group-assignment input { padding:4px 8px; width:220px; max-width:100%; }
.pin-session-note { margin-top:20px; line-height:1.6; }
@media (max-width:640px) { .pin-entry { gap:10px; flex-wrap:wrap; } .pin-thumbnail { width:64px; height:48px; } .pin-entry-actions { margin-left:auto; } }
</style>
