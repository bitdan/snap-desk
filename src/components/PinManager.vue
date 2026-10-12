<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop, onPinsChanged, type PinSummary, type PinManageAction } from '@/lib/api'
import PinCreator from './PinCreator.vue'
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
  <h1>贴图</h1><p class="subtitle">把截图、图片文件、文本或颜色贴在桌面上，边查看边工作。</p>
  <div v-if="error" class="notice error" role="alert">{{ error }}</div>
  <div class="pin-manager-actions">
    <Button :disabled="!desktop || busy" @click="run(api.pinClipboard)">剪贴板图片贴图</Button>
    <Button variant="outline" :disabled="!desktop || busy" @click="run(api.pinFile)">打开图片贴图</Button>
  </div>
  <PinCreator :disabled="!desktop || busy" @create="run" />
  <div class="pin-manager-actions group-actions">
    <label>分组 <select v-model="groupFilter" :disabled="busy" aria-label="筛选贴图分组"><option :value="null">全部贴图</option><option value="">未分组</option><option v-for="group in groups" :key="group" :value="group">{{ group }}</option></select></label>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('show')">{{ groupFilter === null ? '全部' : '本组' }}显示</Button>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('hide')">{{ groupFilter === null ? '全部' : '本组' }}隐藏</Button>
    <Button variant="outline" :disabled="!desktop || busy || !visiblePins.length" @click="manage('close')">{{ groupFilter === null ? '全部' : '本组' }}关闭</Button>
  </div>
  <section class="settings-card" aria-label="当前贴图">
    <p v-if="!visiblePins.length" class="pin-empty">{{ pins.length ? '当前分组没有贴图。' : '还没有贴图。可从截图工具栏或上方入口创建。' }}</p>
    <div v-for="pin in visiblePins" :key="pin.label" class="setting-row">
      <label class="pin-choice"><input v-model="selected" type="checkbox" :value="pin.label" :disabled="busy" :aria-label="`选择 ${pin.label}`" /><span>{{ pin.label }}<small>{{ pin.width }} × {{ pin.height }} px · {{ pin.visible ? '显示中' : '已隐藏' }} · {{ pin.group || '未分组' }}</small></span></label>
      <div class="pin-manager-actions"><Button variant="outline" :disabled="busy" @click="manage(pin.visible ? 'hide' : 'show', pin.label)">{{ pin.visible ? '隐藏' : '显示' }}</Button><Button variant="ghost" :disabled="busy" @click="manage('close', pin.label)">关闭</Button></div>
    </div>
  </section>
  <form v-if="pins.length" class="pin-manager-actions group-actions assignment" @submit.prevent="assignGroup">
    <span>已选 {{ selected.length }} 张</span>
    <input v-model="groupName" list="pin-groups" :disabled="busy" placeholder="分组名称，留空移出分组" aria-label="目标分组" />
    <datalist id="pin-groups"><option v-for="group in groups" :key="group" :value="group" /></datalist>
    <Button type="submit" variant="outline" :disabled="!desktop || busy || !selected.length">应用分组</Button>
  </form>
  <section class="operation-list">
    <h2>贴图操作</h2>
    <div><span>移动 / 缩放</span><span>拖动图片 / 滚轮</span></div>
    <div><span>编辑标注 / 更多操作</span><span>图片右键菜单</span></div>
    <div><span>显示 / 收起工具</span><span>Tab · 工具按钮</span></div>
    <div><span>复制 / 保存 PNG</span><kbd>Ctrl / ⌘ + C / S</kbd></div>
    <div><span>隐藏 / 关闭</span><kbd>Esc / Ctrl / ⌘ + W</kbd></div>
    <div><span>图片处理</span><span>旋转 · 翻转 · 灰度 · 反色 · 恢复原图</span></div>
  </section>
  <p class="scope-note">最多 12 张、单张 1600 万像素、总计 3200 万像素；支持 PNG / JPEG / WebP，文件最多 64 MiB，动画 WebP 仅取首帧。分组名称最多 32 字符，空组自动消失。截图和文本贴图保留可编辑标注。关闭窗口或退出应用会清除内存图片及分组，请先保存需要保留的内容。</p>
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
