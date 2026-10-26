<script setup lang="ts">
import { t, te } from '@/lib/i18n'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogTrigger } from 'reka-ui'
import { api, desktop, onClipboardChanged, onClipboardError, onClipboardState, onClipboardOpen } from '@/lib/api'
import { defaultClipboardSettings, toggleSelection, type ClipboardItem as Item, type ClipboardStatus, type ClipboardKind, type ClipboardExportOptions } from '@/lib/clipboard'
import { Search, ClipboardList, Type, Image, Star, X, Download, Pin, Trash2, Files } from 'lucide-vue-next'
import ClipboardItem from './ClipboardItem.vue'
import Button from '@/components/ui/button/Button.vue'
const items = ref<Item[]>([]), selected = ref<number[]>([]), cursor = ref<string | null>(null)
const selectedItems = ref<Record<number, Item>>({})
const status = ref<ClipboardStatus | null>(null), settings = ref(defaultClipboardSettings())
const keyword = ref(''), category = ref<'' | ClipboardKind | 'favorites'>('')
const kind = computed(() => category.value === 'favorites' ? '' : category.value)
const favorites = computed(() => category.value === 'favorites')
const categories = [{ value: '' as const, label: '全部', icon: ClipboardList }, { value: 'text' as const, label: '文本', icon: Type }, { value: 'image' as const, label: '图像', icon: Image }, { value: 'file' as const, label: '文件', icon: Files }, { value: 'favorites' as const, label: '收藏', icon: Star }]
const canExport = computed(() => selected.value.length > 0 && selected.value.every(id => ['text', 'image'].includes(selectedItems.value[id]?.kind)))
const busy = ref(false), loading = ref(false), error = ref(''), message = ref('')
const confirm = ref<'delete' | 'clear' | null>(null), includeFavorites = ref(false), exportOpen = ref(false)
/** 事件中的条件分支放在脚本内，避免被模板编译器当成 JavaScript 表达式解析。 */
function updateExportOpen(open: boolean) {
  if (!busy.value) exportOpen.value = open
}
function guardExportDismiss(event: Event) {
  if (busy.value) event.preventDefault()
}
const options = ref<Omit<ClipboardExportOptions, 'ids'>>({ mode: 'pdf', pageSize: 'A4', gapPx: 16, marginMm: 12, background: 'white' })
const searchInput = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)
function focusSearch() { searchInput.value?.focus(); searchInput.value?.select() }
/** 搜索与列表共用方向键；条目焦点与批量选择独立，浏览不会改变导出顺序。 */
function navigate(event: KeyboardEvent) {
  if (event.isComposing || confirm.value || exportOpen.value) return
  const target = event.target as HTMLElement
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
    event.preventDefault(); focusSearch(); return
  }
  if (event.key === 'Escape') { event.preventDefault(); keyword.value = ''; focusSearch(); return }
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return
  const fromSearch = target === searchInput.value
  if (fromSearch && !['ArrowDown', 'ArrowUp'].includes(event.key)) return
  const row = target.closest<HTMLElement>('.history-item')
  if (!fromSearch && !row) return
  const rows = Array.from(list.value?.querySelectorAll<HTMLElement>('.history-item') ?? [])
  if (!rows.length) return
  event.preventDefault()
  const current = row ? rows.indexOf(row) : -1
  const index = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1
    : current < 0 ? (event.key === 'ArrowDown' ? 0 : rows.length - 1)
    : Math.max(0, Math.min(rows.length - 1, current + (event.key === 'ArrowDown' ? 1 : -1)))
  rows[index].focus(); rows[index].scrollIntoView({ block: 'nearest' })
}
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
// 输入停顿后查询；分类立即生效，清除旧选择和旧请求，避免显示过期结果。
function applyFilter() {
  clearTimeout(refreshTimer)
  selected.value = []; selectedItems.value = {}; void refresh()
}
watch(keyword, () => {
  revision++; clearTimeout(refreshTimer)
  refreshTimer = setTimeout(applyFilter, 250)
})
watch(category, applyFilter)
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
async function remove() {
  const action = confirm.value
  await run(async () => {
    if (action === 'clear') await api.clipboardClear(includeFavorites.value)
    else await api.clipboardDelete(selected.value)
    selected.value = []; selectedItems.value = {}; confirm.value = null; await refresh()
  }, '历史条目已删除。')
}
async function exportItems() {
  if (!canExport.value) return
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
  await nextTick(); focusSearch()
  if (!desktop) return
  try {
    for (const subscribe of [() => onClipboardOpen(focusSearch), () => onClipboardChanged(scheduleRefresh), () => onClipboardError(value => { error.value = value }), () => onClipboardState(value => { status.value = value })]) {
      const off = await subscribe(); if (disposed) { off(); return }; disposers.push(off)
    }
    settings.value = (await api.status()).settings.clipboard
    await refresh()
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; revision++; clearTimeout(refreshTimer); disposers.forEach(off => off()) })
</script>
<template>
  <!-- 复用模态组件管理焦点与遮罩；导出期间保持弹窗，避免重复提交或误关闭。 -->
  <DialogRoot :open="exportOpen" @update:open="updateExportOpen">
  <section class="clipboard-history" @keydown="navigate">
    <header class="history-header">
      <span class="history-count">{{ status?.count ?? 0 }} {{ t("条 ·") }} {{ ((status?.byteSize ?? 0) / 1048576).toFixed(1) }} MiB</span>
      <div class="history-header-actions"><Button variant="ghost" :disabled="busy || loading || !desktop" @click="refresh()">{{ t("刷新") }}</Button><Button variant="ghost" :disabled="busy || !status?.count" @click="confirm = 'clear'; includeFavorites = false">{{ t("清空") }}</Button></div>
      <label class="history-recording"><span>{{ status?.enabled ? t("正在记录") : t("已暂停") }}</span><input type="checkbox" role="switch" :aria-label='t("记录剪贴板历史")' :checked="status?.enabled ?? false" :disabled="busy || !desktop || !status?.supported" @click.prevent="configure(!status?.enabled)" /><span class="recording-track" aria-hidden="true"></span></label>
    </header>
    <p v-if="status && !status.supported" class="notice">{{ t("当前平台尚未支持自动监听；现有手动贴图不受影响。") }}</p>
    <p v-if="error || status?.error" class="notice error" role="alert">{{ te(error || status?.error) }}</p>
    <p v-if="message" role="status">{{ t(message) }}</p>
    <form class="history-search" role="search" @submit.prevent="applyFilter">
      <Search :size="16" aria-hidden="true" /><input ref="searchInput" v-model="keyword" :aria-label='t("搜索文本或文件路径")' :placeholder='t("搜索文本或文件路径（Ctrl+F）")' maxlength="200" /><button v-if="keyword" type="button" :aria-label='t("清除搜索")' @click="keyword = ''; focusSearch()"><X :size="16" /></button>
    </form>
    <nav class="history-categories" :aria-label='t("历史分类")'>
      <button v-for="tab in categories" :key="tab.value" :class="{ active: category === tab.value }" :aria-pressed="category === tab.value" :disabled="busy || !desktop" @click="category = tab.value"><component :is="tab.icon" :size="16" aria-hidden="true" />{{ t(tab.label) }}</button>
    </nav>
    <div v-if="confirm" class="notice" role="alertdialog" :aria-label='t("确认删除历史")'>
      <p>{{ confirm === 'clear' ? t("清空历史记录？默认保留收藏。") : t("删除选中的 {0} 条记录（包含选中的收藏）？", { 0: selected.length }) }} {{ t("此操作不可撤销。") }}</p>
      <label v-if="confirm === 'clear'"><input v-model="includeFavorites" type="checkbox" />{{ t("同时删除收藏") }}</label>
      <Button :disabled="busy" @click="remove">{{ t("确认删除") }}</Button><Button variant="outline" :disabled="busy" @click="confirm = null">{{ t("取消") }}</Button>
    </div>
    <DialogPortal>
    <DialogOverlay class="history-export-overlay" />
    <DialogContent class="history-export-dialog" @keydown.stop @escape-key-down="guardExportDismiss" @interact-outside="guardExportDismiss">
      <DialogTitle class="history-export-title">{{ t("导出") }} {{ selected.length }} {{ t("个素材") }}</DialogTitle>
      <DialogDescription class="history-export-description">{{ t("按选择顺序导出，图片使用原图。") }}</DialogDescription>
      <fieldset :disabled="busy">
      <div class="history-tools"><select v-model="options.mode" :aria-label='t("导出方式")'><option value="pdf">{{ t("多页 PDF") }}</option><option value="longPng">{{ t("纵向长图 PNG") }}</option><option value="separate">{{ t("分别导出 PNG / TXT") }}</option></select><select v-if="options.mode === 'pdf'" v-model="options.pageSize" :aria-label='t("页面大小")'><option value="A4">A4</option><option value="original">{{ t("图片原始比例") }}</option></select><label v-if="options.mode === 'pdf'">{{ t("边距 mm") }}<input v-model.number="options.marginMm" type="number" min="0" max="30" /></label><label v-if="options.mode === 'longPng'">{{ t("间距 px") }}<input v-model.number="options.gapPx" type="number" min="0" max="200" /></label><select v-if="options.mode === 'longPng'" v-model="options.background" :aria-label='t("背景")'><option value="white">{{ t("白底") }}</option><option value="transparent">{{ t("透明背景") }}</option></select></div>
      </fieldset>
      <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
      <p v-else-if="message" role="status">{{ t(message) }}</p>
      <div class="history-export-actions"><Button :disabled="busy || !selected.length" @click="exportItems">{{ busy ? t("正在导出…") : t("选择位置并导出") }}</Button><Button variant="outline" :disabled="busy" @click="exportOpen = false">{{ t("关闭") }}</Button></div>
    </DialogContent>
    </DialogPortal>
    <div class="history-browser">
    <div ref="list" class="history-list" role="region" :aria-label='t("历史记录列表")' :aria-busy="loading">
      <ClipboardItem v-for="item in items" :key="item.id" :item="item" :selected="selected.includes(item.id)" :busy="busy" @select="selectItem(item)" @restore="run(() => api.clipboardRestore(item.id), '已复制。')" @delete="selected = [item.id]; selectedItems = { [item.id]: item }; confirm = 'delete'" @pin="run(() => api.clipboardPin(item.id))" @favorite="run(async () => { await api.clipboardFavorite(item.id, !item.favorite); await refresh() })" />
      <p v-if="!items.length" class="history-empty">{{ loading ? t("正在加载…") : keyword || category ? t("没有匹配的记录，试试其他关键词或分类。") : status?.enabled ? t("暂无记录，复制文字或图片后会显示在这里。") : t("暂无记录，开启右上方开关后开始记录。") }}</p>
      <Button v-if="cursor" variant="outline" :disabled="loading" @click="refresh(true)">{{ loading ? t("加载中…") : t("加载更多") }}</Button>
    </div>
    <!-- 侧栏独立于列表滚动并始终预留宽度，选择条目不会挤动列表或影响双击。 -->
    <aside class="history-action-rail" :aria-label='t("所选条目操作")'>
      <template v-if="selected.length">
        <span class="history-selected-count" role="status" :title="t('已选 {0} / 100 条', { 0: selected.length })">{{ t("已选") }}<br />{{ selected.length }}</span>
        <DialogTrigger as-child><Button class="history-action" :disabled="busy || !canExport" :title="canExport ? t('组合导出') : t('组合导出仅支持文本和图片，文件引用请复制后粘贴')" :aria-label='t("组合导出所选条目")' @click="error = ''; message = ''"><Download :size="18" /></Button></DialogTrigger>
        <Button class="history-action" variant="outline" :disabled="busy || !selected.some(id => selectedItems[id]?.kind === 'image')" :title='t("图片贴图")' :aria-label='t("将所选图片贴图")' @click="pinSelected"><Pin :size="18" /></Button>
        <Button class="history-action history-delete" variant="outline" :disabled="busy" :title='t("删除所选")' :aria-label='t("删除所选条目")' @click="confirm = 'delete'"><Trash2 :size="18" /></Button>
        <Button class="history-action" variant="ghost" :disabled="busy" :title='t("取消选择")' :aria-label='t("取消选择")' @click="selected = []; selectedItems = {}"><X :size="18" /></Button>
      </template>
    </aside>
    </div>
    <footer class="history-footer"><span>{{ t("单击选择 · ↑↓ 浏览 · Enter / 双击复制") }}</span><p v-if="category === 'file'">{{ t("文件仅保存路径引用，不备份内容；原文件移动或删除后无法恢复。复制后可到资源管理器粘贴。") }}</p><p>{{ t("历史仅存本机，未加密、不上传，可能包含密码等私人内容。暂停不影响已有记录。") }}</p></footer>
  </section>
  </DialogRoot>
</template>
<style>
.history-export-overlay { position:fixed; inset:0; z-index:110; background:#0006; }
.history-export-dialog { position:fixed; z-index:111; top:50%; left:50%; transform:translate(-50%,-50%); width:min(600px, calc(100vw - 32px)); max-height:calc(100vh - 32px); overflow-y:auto; border:1px solid #d8dee4; border-radius:6px; background:#fff; box-shadow:0 12px 40px #0003; font-size:13px; }
.history-export-dialog { padding:20px; }
.history-export-title { font-size:18px; font-weight:600; }
.history-export-description { margin:8px 0 16px; color:#57606a; font-size:12px; }
.history-export-dialog input, .history-export-dialog select { border:1px solid #d4d4d8; border-radius:4px; padding:6px; max-width:100%; }
.history-export-dialog input[type=number] { width:90px; }
.history-export-actions { display:flex; justify-content:flex-end; gap:8px; margin-top:16px; }
.clipboard-history { display:flex; flex-direction:column; flex:1; min-height:0; font-size:13px; gap:8px; overflow-y:auto; }
.clipboard-history > * { flex-shrink:0; }
.clipboard-history .history-browser { display:flex; flex:1 1 160px; min-height:160px; gap:8px; }
.history-browser .history-list { flex:1; min-width:0; overflow-y:auto; overscroll-behavior:contain; }
.history-action-rail { display:flex; flex-direction:column; align-items:center; gap:12px; width:48px; flex-shrink:0; padding:8px 4px; overflow-y:auto; }
.history-action-rail .history-action { width:36px; height:36px; min-height:36px; padding:0; border-radius:50%; }
.history-selected-count { font-size:12px; color:#57606a; text-align:center; line-height:1.5; }
.history-action-rail .history-delete { margin-top:auto; }
.history-count { color:#57606a; font-size:12px; white-space:nowrap; }
.history-header-actions { display:flex; margin-left:auto; }
.history-header-actions button { padding:4px 8px; height:28px; font-size:12px; }
.history-footer { border-top:1px solid #e1e4e8; padding-top:8px; font-size:12px; color:#57606a; line-height:1.6; }
.history-footer p { margin-top:4px; color:#71717a; }
.history-empty { padding:32px 16px; text-align:center; color:#71717a; }
.history-tools { display:flex; flex-wrap:wrap; gap:10px; align-items:center; margin:12px 0; }
.history-header { display:flex; align-items:center; justify-content:space-between; gap:8px; flex-wrap:wrap; }
.history-header h1 { margin-bottom:4px; }.history-header .subtitle { margin:0; }
.history-recording { display:flex; align-items:center; gap:8px; font-size:13px; position:relative; cursor:pointer; }
.history-recording input { position:absolute; right:0; width:38px; height:22px; opacity:0; }
.recording-track { width:38px; height:22px; border-radius:12px; background:#a1a1aa; transition:background .15s; pointer-events:none; }
.recording-track::after { content:''; display:block; width:18px; height:18px; margin:2px; border-radius:50%; background:white; transition:transform .15s; }
.history-recording input:checked + .recording-track { background:#2563eb; }
.history-recording input:checked + .recording-track::after { transform:translateX(16px); }
.history-recording input:focus-visible + .recording-track { outline:2px solid #2563eb; outline-offset:3px; }
.history-recording input:disabled + .recording-track { opacity:.45; }
.clipboard-history input:not([type=checkbox]), .clipboard-history select { border:1px solid #d4d4d8; border-radius:6px; padding:6px; max-width:220px; }
.clipboard-history input[type=number] { width:90px; }
.history-item { display:flex; gap:8px; align-items:center; padding:8px; margin:0; border:1px solid transparent; border-bottom-color:#e4e4e7; border-radius:4px; transition:background .15s, box-shadow .15s; }
.history-item:hover { background:#f8fafc; }
.history-item.is-selected { background:#eff6ff; border-color:#93b4ff; box-shadow:0 2px 8px #2563eb26; }
.history-item:focus-within { border-color:#93b4ff; }
.history-item:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.history-content { flex:1; min-width:0; text-align:left; }.history-content img { max-width:100%; max-height:96px; object-fit:contain; }
.history-text { white-space:pre-wrap; overflow-wrap:anywhere; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; line-height:1.5; }
.history-meta { display:flex; flex-wrap:wrap; justify-content:space-between; gap:4px 12px; color:#71717a; margin-top:4px; font-size:12px; }
.history-content img { display:block; margin:0 auto; box-shadow:0 1px 5px #0002; }
.history-search { display:flex; align-items:center; gap:8px; color:#71717a; padding:2px 8px; border:1px solid #d8dee4; border-radius:4px; }
.history-search > svg { flex-shrink:0; }
.history-search button { display:flex; align-items:center; justify-content:center; min-width:24px; min-height:24px; }
.clipboard-history .history-search input { border:0; outline:none; max-width:none; min-width:0; width:100%; background:transparent; }
.history-search:focus-within { box-shadow:0 1px #2563eb; }
.history-categories { display:flex; gap:8px; border-bottom:1px solid #e4e4e7; }
.history-categories button { display:flex; gap:8px; align-items:center; justify-content:center; padding:10px 20px; border-bottom:2px solid transparent; color:#71717a; }
.history-categories button.active { color:#2563eb; border-bottom-color:#2563eb; background:#eff6ff; }
@media (max-width:600px) { .history-categories { gap:0; }.history-categories button { flex:1; padding:10px 8px; } }
.clipboard-history pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:360px; overflow:auto; }
</style>
