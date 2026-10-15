<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, desktop, onClipboardChanged, onClipboardError, onClipboardState } from '@/lib/api'
import { defaultClipboardSettings, toggleSelection, reorderSelection, type ClipboardItem as Item, type ClipboardStatus, type ClipboardKind, type ClipboardExportOptions } from '@/lib/clipboard'
import ClipboardItem from './ClipboardItem.vue'
import Button from '@/components/ui/button/Button.vue'
const items = ref<Item[]>([]), selected = ref<number[]>([]), cursor = ref<string | null>(null)
const selectedItems = ref<Record<number, Item>>({})
const status = ref<ClipboardStatus | null>(null), settings = ref(defaultClipboardSettings())
const keyword = ref(''), kind = ref<'' | ClipboardKind>(''), favorites = ref(false)
const busy = ref(false), loading = ref(false), error = ref(''), message = ref(''), preview = ref<{ item: Item; content: string } | null>(null)
const confirm = ref<'delete' | 'clear' | null>(null), includeFavorites = ref(false), exportOpen = ref(false)
const options = ref<Omit<ClipboardExportOptions, 'ids'>>({ mode: 'pdf', pageSize: 'A4', gapPx: 16, marginMm: 12, background: 'white' })
const ordered = computed(() => selected.value.map(id => ({ id, label: selectedItems.value[id]?.preview || `图片 / 条目 ${id}` })))
function selectItem(item: Item) {
  selected.value = toggleSelection(selected.value, item.id)
  selectedItems.value = Object.fromEntries(selected.value.map(id => [id, id === item.id ? item : selectedItems.value[id]]))
}
let revision = 0, disposed = false, refreshTimer: ReturnType<typeof setTimeout> | undefined
const disposers: (() => void)[] = []
async function refresh(append = false) {
  if (!desktop || disposed) return
  const request = ++revision; loading.value = true
  try {
    const [page, state] = await Promise.all([api.clipboardList({ keyword: keyword.value, kind: kind.value || undefined, favoritesOnly: favorites.value, limit: 40, cursor: append ? cursor.value ?? undefined : undefined }), api.clipboardStatus()])
    if (disposed || request !== revision) return
    const seen = new Set(append ? items.value.map(item => item.id) : [])
    items.value = [...(append ? items.value : []), ...page.items.filter(item => !seen.has(item.id))]
    cursor.value = page.nextCursor; status.value = state
  } catch (e) { if (!disposed && request === revision) error.value = String(e) }
  finally { if (request === revision) loading.value = false }
}
function scheduleRefresh() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => { void refresh() }, 100)
}
async function run(action: () => Promise<unknown>, success = '') {
  if (busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await action(); if (success) message.value = success }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function configure(enabled = settings.value.enabled) {
  await run(async () => {
    status.value = await api.clipboardConfigure({ ...settings.value, enabled })
    settings.value = (await api.status()).settings.clipboard
  }, enabled ? '历史记录已开启，仅记录此后复制的内容。' : '历史记录已暂停。')
}
async function showPreview(item: Item) {
  await run(async () => { preview.value = { item, content: item.kind === 'text' ? await api.clipboardText(item.id) : await api.clipboardThumbnail(item.id) } })
}
async function remove() {
  const action = confirm.value
  await run(async () => {
    if (action === 'clear') await api.clipboardClear(includeFavorites.value)
    else await api.clipboardDelete(selected.value)
    selected.value = []; preview.value = null; confirm.value = null; await refresh()
  }, '历史条目已删除。')
}
async function exportItems() {
  await run(async () => {
    const result = await api.clipboardExport({ ...options.value, ids: selected.value })
    message.value = result.status === 'cancelled' ? '已取消导出。' : `已保存 ${result.outputPaths.length} 个文件。`
    if (result.status === 'saved') exportOpen.value = false
  })
}
async function pinSelected() {
  await run(async () => {
    let done = 0
    for (const id of selected.value) {
      if (selectedItems.value[id]?.kind !== 'image') continue
      try { await api.clipboardPin(id); done++ } catch (e) { throw new Error(`已创建 ${done} 张贴图；${String(e)}`) }
    }
    message.value = `已创建 ${done} 张图片贴图。`
  })
}
onMounted(async () => {
  if (!desktop) return
  try {
    for (const subscribe of [() => onClipboardChanged(scheduleRefresh), () => onClipboardError(value => { error.value = value }), () => onClipboardState(value => { status.value = value })]) {
      const off = await subscribe(); if (disposed) { off(); return }; disposers.push(off)
    }
    settings.value = (await api.status()).settings.clipboard
    await refresh()
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; revision++; clearTimeout(refreshTimer); disposers.forEach(off => off()) })
</script>
<template>
  <section class="clipboard-history">
    <h1>剪贴板历史</h1>
    <p class="subtitle">{{ status?.enabled ? '正在记录' : '已暂停' }} · {{ status?.count ?? 0 }} 条 · {{ ((status?.byteSize ?? 0) / 1048576).toFixed(1) }} MiB</p>
    <p class="notice">开启后，复制的文字和图片将保存在本机，可能包含密码、令牌等私人内容。数据未加密，不会上传；暂停后仍可查看、复制和清空已有历史。</p>
    <p v-if="status && !status.supported" class="notice">当前平台尚未支持自动监听；现有手动贴图不受影响。</p>
    <p v-if="error || status?.error" class="notice error" role="alert">{{ error || status?.error }}</p>
    <p v-if="message" role="status">{{ message }}</p>
    <div class="history-tools">
      <Button :disabled="busy || !desktop || !status?.supported" @click="configure(!status?.enabled)">{{ status?.enabled ? '立即暂停' : '开启记录 / 重试' }}</Button>
      <Button variant="outline" :disabled="busy || loading || !desktop" @click="refresh()">刷新</Button>
      <Button variant="outline" :disabled="busy || !status?.count" @click="confirm = 'clear'; includeFavorites = false">清空历史</Button>
    </div>
    <details class="settings-card"><summary>记录与自动清理设置</summary>
      <form @submit.prevent="configure()">
        <div class="setting-row"><label>记录文本</label><input v-model="settings.saveCopiedText" type="checkbox" :disabled="busy" /></div>
        <div class="setting-row"><label>记录图片</label><input v-model="settings.saveCopiedImages" type="checkbox" :disabled="busy" /></div>
        <div class="setting-row"><label for="history-count">最多条目</label><input id="history-count" v-model.number="settings.maxItems" type="number" min="100" max="5000" required /></div>
        <div class="setting-row"><label for="history-size">容量上限（MiB）</label><input id="history-size" v-model.number="settings.maxStorageMb" type="number" min="50" max="2048" required /></div>
        <div class="setting-row"><label for="history-days">保留天数（0 为不限）<small>收藏不会自动删除；启动、保存设置和记录期间自动清理。</small></label><input id="history-days" v-model.number="settings.retainDays" type="number" min="0" max="3650" required /></div>
        <Button type="submit" :disabled="busy || !desktop">保存历史设置</Button>
      </form>
    </details>
    <form class="history-tools" @submit.prevent="selected = []; refresh()">
      <input v-model="keyword" aria-label="搜索文本" placeholder="搜索文本" maxlength="200" />
      <select v-model="kind" aria-label="类型"><option value="">全部类型</option><option value="text">文本</option><option value="image">图片</option></select>
      <label><input v-model="favorites" type="checkbox" />仅收藏</label><Button type="submit" :disabled="loading || !desktop">筛选</Button>
    </form>
    <div class="history-tools"><span>已选 {{ selected.length }} / 100</span><Button variant="outline" :disabled="busy || !selected.length" @click="exportOpen = true">组合导出</Button><Button variant="outline" :disabled="busy || !selected.length" @click="pinSelected">批量图片贴图</Button><Button variant="outline" :disabled="busy || !selected.length" @click="confirm = 'delete'">删除所选</Button><button @click="selected = []">取消选择</button></div>
    <div v-if="confirm" class="notice" role="alertdialog" aria-label="确认删除历史">
      <p>{{ confirm === 'clear' ? '清空历史记录？默认保留收藏。' : `删除选中的 ${selected.length} 条记录（包含选中的收藏）？` }} 此操作不可撤销。</p>
      <label v-if="confirm === 'clear'"><input v-model="includeFavorites" type="checkbox" />同时删除收藏</label>
      <Button :disabled="busy" @click="remove">确认删除</Button><Button variant="outline" :disabled="busy" @click="confirm = null">取消</Button>
    </div>
    <div v-if="exportOpen" class="settings-card export-panel" role="dialog" aria-label="组合导出">
      <h2>导出 {{ selected.length }} 个素材</h2><p>按下列顺序导出，可拖动排序。图片使用历史原图。</p>
      <ol><li v-for="(item, index) in ordered" :key="item.id" draggable="true" @dragstart="$event.dataTransfer?.setData('text/plain', String(index))" @dragover.prevent @drop.prevent="selected = reorderSelection(selected, Number($event.dataTransfer?.getData('text/plain')), index)"><span>{{ index + 1 }}. {{ item.label.slice(0, 50) }}</span><button :disabled="index === 0" @click="selected = reorderSelection(selected, index, index - 1)">上移</button><button :disabled="index === selected.length - 1" @click="selected = reorderSelection(selected, index, index + 1)">下移</button></li></ol>
      <div class="history-tools"><select v-model="options.mode" aria-label="导出方式"><option value="pdf">多页 PDF</option><option value="longPng">纵向长图 PNG</option><option value="separate">分别导出 PNG / TXT</option></select><select v-if="options.mode === 'pdf'" v-model="options.pageSize" aria-label="页面大小"><option value="A4">A4</option><option value="original">图片原始比例</option></select><label v-if="options.mode === 'pdf'">边距 mm<input v-model.number="options.marginMm" type="number" min="0" max="30" /></label><label v-if="options.mode === 'longPng'">间距 px<input v-model.number="options.gapPx" type="number" min="0" max="200" /></label><select v-if="options.mode === 'longPng'" v-model="options.background" aria-label="背景"><option value="white">白底</option><option value="transparent">透明背景</option></select></div>
      <Button :disabled="busy || !selected.length" @click="exportItems">选择位置并导出</Button><Button variant="outline" :disabled="busy" @click="exportOpen = false">关闭</Button>
    </div>
    <aside v-if="preview" class="notice" aria-label="内容预览"><Button variant="ghost" @click="preview = null">关闭预览</Button><pre v-if="preview.item.kind === 'text'">{{ preview.content }}</pre><template v-else><img :src="preview.content" alt="图片预览" /><p>缩略预览 · 原图 {{ preview.item.width }} × {{ preview.item.height }}，可贴图查看原图。</p></template></aside>
    <ClipboardItem v-for="item in items" :key="item.id" :item="item" :selected="selected.includes(item.id)" :busy="busy" @select="selectItem(item)" @preview="showPreview(item)" @restore="run(() => api.clipboardRestore(item.id), '已复制。')" @pin="run(() => api.clipboardPin(item.id))" @favorite="run(async () => { await api.clipboardFavorite(item.id, !item.favorite); await refresh() })" />
    <p v-if="!items.length && !loading">暂无匹配的历史记录。</p><Button v-if="cursor" variant="outline" :disabled="loading" @click="refresh(true)">{{ loading ? '加载中…' : '加载更多' }}</Button>
  </section>
</template>
<style>
.history-tools { display:flex; flex-wrap:wrap; gap:10px; align-items:center; margin:12px 0; }
.clipboard-history input:not([type=checkbox]), .clipboard-history select { border:1px solid #d4d4d8; border-radius:6px; padding:6px; max-width:220px; }
.clipboard-history input[type=number] { width:90px; }
.history-item { display:flex; gap:12px; align-items:center; padding:12px 0; border-bottom:1px solid #e4e4e7; }
.history-content { flex:1; min-width:0; text-align:left; }.history-content img { max-width:240px; max-height:160px; }
.history-text { white-space:pre-wrap; overflow-wrap:anywhere; display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; }
.history-content small { display:block; color:#71717a; margin-top:6px; }.history-actions { display:flex; flex-direction:column; gap:8px; font-size:12px; }
.clipboard-history pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:360px; overflow:auto; }
.export-panel { padding:12px; }.export-panel ol { max-height:220px; overflow:auto; }.export-panel li { display:flex; gap:12px; padding:6px; }.export-panel li span { flex:1; overflow-wrap:anywhere; }
</style>
