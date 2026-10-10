<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop, onPinsChanged, type PinSummary, type PinManageAction } from '@/lib/api'

const pins = ref<PinSummary[]>([])
const busy = ref(false)
const error = ref('')
let disposed = false
let off: (() => void) | undefined
let revision = 0
/** 窗口增删事件可能紧邻到达，只接受最新查询结果，避免旧列表覆盖新状态。 */
async function refresh() {
  const current = ++revision
  try {
    const result = await api.listPins()
    if (!disposed && current === revision) pins.value = result
  } catch (e) { if (!disposed && current === revision) error.value = String(e) }
}
async function run(task: () => Promise<void>) {
  if (busy.value) return
  busy.value = true; error.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { await refresh(); busy.value = false }
}
function manage(action: PinManageAction, label: string | null = null) { void run(() => api.managePins(action, label)) }
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
  <h1>贴图</h1><p class="subtitle">把截图或剪贴板图片贴在桌面上，边查看边工作。</p>
  <div v-if="error" class="notice error" role="alert">{{ error }}</div>
  <div class="pin-manager-actions">
    <Button :disabled="!desktop || busy" @click="run(api.pinClipboard)">剪贴板图片贴图</Button>
    <Button variant="outline" :disabled="!desktop || busy || !pins.length" @click="manage('show')">全部显示</Button>
    <Button variant="outline" :disabled="!desktop || busy || !pins.length" @click="manage('hide')">全部隐藏</Button>
    <Button variant="outline" :disabled="!desktop || busy || !pins.length" @click="manage('close')">全部关闭</Button>
  </div>
  <section class="settings-card" aria-label="当前贴图">
    <p v-if="!pins.length" class="pin-empty">还没有贴图。在截图工具栏点击“贴图”，或复制图片后点击上方按钮。</p>
    <div v-for="pin in pins" :key="pin.label" class="setting-row">
      <span>{{ pin.label }}<small>{{ pin.width }} × {{ pin.height }} px · {{ pin.visible ? '显示中' : '已隐藏' }}</small></span>
      <div class="pin-manager-actions"><Button variant="outline" :disabled="busy" @click="manage(pin.visible ? 'hide' : 'show', pin.label)">{{ pin.visible ? '隐藏' : '显示' }}</Button><Button variant="ghost" :disabled="busy" @click="manage('close', pin.label)">关闭</Button></div>
    </div>
  </section>
  <section class="operation-list">
    <h2>贴图操作</h2>
    <div><span>移动 / 缩放</span><span>拖动图片 / 滚轮</span></div>
    <div><span>编辑标注 / 更多操作</span><span>图片右键菜单</span></div>
    <div><span>显示 / 收起工具</span><span>Tab · 工具按钮</span></div>
    <div><span>复制 / 保存 PNG</span><kbd>Ctrl / ⌘ + C / S</kbd></div>
    <div><span>隐藏 / 关闭</span><kbd>Esc / Ctrl / ⌘ + W</kbd></div>
    <div><span>图片处理</span><span>旋转 · 翻转 · 灰度 · 反色 · 恢复原图</span></div>
  </section>
  <p class="scope-note">最多 12 张、单张 1600 万像素、总计 3200 万像素。底部直接选择标注工具，指针工具可修改已有标注；截图转贴图会保留可编辑图形。关闭窗口或退出应用会清除内存图片，请先保存需要保留的内容。</p>
</template>

<style scoped>
.pin-manager-actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 0 0 16px; }
.setting-row .pin-manager-actions { margin: 0; }
.pin-empty { padding: 24px 0; color: #6e7781; font-size: 13px; }
</style>
