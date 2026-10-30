<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import { t, te } from '@/shared/i18n/i18n'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription, DialogTrigger } from 'reka-ui'
import { PopoverRoot, PopoverTrigger, PopoverPortal, PopoverContent } from 'reka-ui'
import { api, desktop, onClipboardChanged, onClipboardError, onClipboardState, onClipboardOpen } from '@/services/api'
import { defaultClipboardSettings, selectClipboardItems, type ClipboardSelectionGesture, type ClipboardItem as Item, type ClipboardStatus, type ClipboardKind, type ClipboardExportOptions } from '@/features/clipboard/clipboard'
import { Search, ClipboardList, Type, Image, Star, X, Download, Pin, Trash2, Files, List, LayoutGrid, Ellipsis, LoaderCircle } from 'lucide-vue-next'
import ClipboardItem from '@/features/clipboard/components/ClipboardItem.vue'
import VirtualClipboardList from '@/features/clipboard/components/VirtualClipboardList.vue'
import ClipboardPreview from '@/features/clipboard/components/ClipboardPreview.vue'
import { clipboardThumbnails } from '@/features/clipboard/clipboard-thumbnails'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape, handleWindowEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
const items = ref<Item[]>([]), selected = ref<number[]>([]), cursor = ref<string | null>(null)
const selectedItems = ref<Record<number, Item>>({})
const previewItem = ref<Item | null>(null)
let selectionAnchor: number | null = null
const viewKey = 'snap-desk.clipboard-view.v1'
const layout = ref<'list' | 'grid'>((() => {
  try { return localStorage.getItem(viewKey) === 'grid' ? 'grid' : 'list' } catch { return 'list' }
})())
function setLayout(value: 'list' | 'grid') {
  layout.value = value
  try { localStorage.setItem(viewKey, value) } catch { /* 存储不可用时仍允许当前窗口切换。 */ }
}
async function closePreview() {
  const index = items.value.findIndex(item => item.id === previewItem.value?.id)
  previewItem.value = null
  await nextTick()
  if (index >= 0) await list.value?.focusIndex(index)
}
const status = ref<ClipboardStatus | null>(null), settings = ref(defaultClipboardSettings())
const keyword = ref(''), category = ref<'' | ClipboardKind | 'favorites'>('')
const kind = computed(() => category.value === 'favorites' ? '' : category.value)
const favorites = computed(() => category.value === 'favorites')
const categories = [{ value: '' as const, label: '全部', icon: ClipboardList }, { value: 'text' as const, label: '文本', icon: Type }, { value: 'image' as const, label: '图像', icon: Image }, { value: 'file' as const, label: '文件', icon: Files }, { value: 'favorites' as const, label: '收藏', icon: Star }]
const canExport = computed(() => selected.value.length > 0 && selected.value.every(id => ['text', 'image'].includes(selectedItems.value[id]?.kind)))
const busy = ref(false), loading = ref(false), error = ref(''), message = ref('')
const foregroundLoading = ref(false), loadingMore = ref(false)
const confirm = ref<'delete' | 'clear' | null>(null), includeFavorites = ref(false), exportOpen = ref(false)
const helpOpen = ref(false)
useEscapeLayer(() => !!confirm.value, () => { if (!busy.value) confirm.value = null })
useEscapeLayer(() => exportOpen.value, () => { if (!busy.value) exportOpen.value = false })
useEscapeLayer(() => helpOpen.value, () => { helpOpen.value = false })
function exportKey(event: KeyboardEvent) { handleEscape(event, () => { if (!busy.value) exportOpen.value = false }) }
function helpKey(event: KeyboardEvent) { handleEscape(event, () => { helpOpen.value = false }) }
/** 事件中的条件分支放在脚本内，避免被模板编译器当成 JavaScript 表达式解析。 */
function updateExportOpen(open: boolean) {
  if (!busy.value) exportOpen.value = open
}
function guardExportDismiss(event: Event) {
  if (busy.value) event.preventDefault()
}
const options = ref<Omit<ClipboardExportOptions, 'ids'>>({ mode: 'pdf', pageSize: 'A4', gapPx: 16, marginMm: 12, background: 'white' })
const searchInput = ref<HTMLInputElement | null>(null)
const list = ref<InstanceType<typeof VirtualClipboardList> | null>(null)
function focusSearch() { searchInput.value?.focus(); searchInput.value?.select() }
/** 搜索与列表共用方向键；条目焦点与批量选择独立，浏览不会改变导出顺序。 */
function navigate(event: KeyboardEvent) {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || previewItem.value) return
  if (confirm.value || exportOpen.value || helpOpen.value) {
    handleWindowEscape(event, () => {})
    return
  }
  const target = event.target as HTMLElement
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
    event.preventDefault(); focusSearch(); return
  }
  if (event.key === 'Escape') {
    if (selected.value.length || keyword.value) handleEscape(event, () => {
      if (selected.value.length) { selected.value = []; selectedItems.value = {}; selectionAnchor = null }
      else { keyword.value = ''; focusSearch() }
    })
    return
  }
  const directions = layout.value === 'grid' ? ['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End'] : ['ArrowDown', 'ArrowUp', 'Home', 'End']
  if (!directions.includes(event.key) || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return
  const fromSearch = target === searchInput.value
  if (fromSearch && !['ArrowDown', 'ArrowUp'].includes(event.key)) return
  const row = target.closest<HTMLElement>('.history-item')
  if (!fromSearch && !row) return
  if (!items.value.length) return
  event.preventDefault()
  const current = row ? Number(row.closest<HTMLElement>('[data-history-index]')?.dataset.historyIndex ?? -1) : -1
  const step = layout.value === 'grid' && ['ArrowDown', 'ArrowUp'].includes(event.key) ? list.value?.columns ?? 1 : 1
  const index = event.key === 'Home' ? 0 : event.key === 'End' ? items.value.length - 1
    : current < 0 ? (event.key === 'ArrowDown' ? 0 : items.value.length - 1)
    : Math.max(0, Math.min(items.value.length - 1, current + (['ArrowDown', 'ArrowRight'].includes(event.key) ? step : -step)))
  void list.value?.focusIndex(index)
}
function selectItem(item: Item, gesture: ClipboardSelectionGesture = { toggle: true }) {
  selected.value = selectClipboardItems(selected.value, item.id, items.value.map(value => value.id), selectionAnchor, gesture)
  if (!gesture.range) selectionAnchor = item.id
  const available = new Map(items.value.map(value => [value.id, value]))
  selectedItems.value = Object.fromEntries(selected.value.map(id => [id, available.get(id) ?? selectedItems.value[id]]))
}
let revision = 0, disposed = false, refreshTimer: ReturnType<typeof setTimeout> | undefined
const disposers: (() => void)[] = []
async function refresh(append = false, background = false) {
  if (!desktop || disposed) return
  const request = ++revision; loading.value = true
  foregroundLoading.value = !background; loadingMore.value = append
  try {
    const [page, state] = await Promise.all([api.clipboardList({ keyword: keyword.value, kind: kind.value || undefined, favoritesOnly: favorites.value, limit: 40, cursor: append ? cursor.value ?? undefined : undefined }), api.clipboardStatus()])
    if (disposed || request !== revision) return
    const seen = new Set(append ? items.value.map(item => item.id) : [])
    items.value = [...(append ? items.value : []), ...page.items.filter(item => !seen.has(item.id))]
    cursor.value = page.nextCursor; status.value = state
  } catch (e) { if (!disposed && request === revision) error.value = String(e) }
  finally { if (request === revision) { loading.value = false; foregroundLoading.value = false; loadingMore.value = false } }
}
function scheduleRefresh() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => {
    // 内容变更静默更新；用户正在查询时延后，避免后台事件抢占前台加载状态。
    if (loading.value) { scheduleRefresh(); return }
    void refresh(false, true)
  }, 100)
}
// 输入停顿后查询；分类立即生效，清除旧选择和旧请求，避免显示过期结果。
function applyFilter() {
  clearTimeout(refreshTimer)
  selected.value = []; selectedItems.value = {}; selectionAnchor = null; list.value?.resetScroll(); void refresh()
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
    clipboardThumbnails.clear()
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
      if (!['image', 'text'].includes(selectedItems.value[id]?.kind)) continue
      try { await api.clipboardPin(id); done++ } catch (e) { throw new Error(`已创建 ${done} 张贴图；${String(e)}`) }
    }
    message.value = `已创建 ${done} 张贴图。`
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
onUnmounted(() => { disposed = true; revision++; clearTimeout(refreshTimer); disposers.forEach(off => off()); clipboardThumbnails.clear() })
</script>
<template>
  <!-- 复用模态组件管理焦点与遮罩；导出期间保持弹窗，避免重复提交或误关闭。 -->
  <DialogRoot :open="exportOpen" @update:open="updateExportOpen">
  <section class="clipboard-history" @keydown="navigate">
    <header class="history-header">
      <span class="history-count">{{ status?.count ?? 0 }} {{ t("条 ·") }} {{ ((status?.byteSize ?? 0) / 1048576).toFixed(1) }} MiB</span>
      <div class="history-header-actions"><Button variant="ghost" :disabled="busy || foregroundLoading || !desktop" @click="refresh()">{{ t("刷新") }}</Button><Button variant="ghost" class="history-clear" :disabled="busy || !status?.count" @click="confirm = 'clear'; includeFavorites = false">{{ t("清空") }}</Button></div>
      <label class="history-recording"><span>{{ status?.enabled ? t("正在记录") : t("已暂停") }}</span><input type="checkbox" role="switch" :aria-label='t("记录剪贴板历史")' :checked="status?.enabled ?? false" :disabled="busy || !desktop || !status?.supported" @click.prevent="configure(!status?.enabled)" /><span class="recording-track" aria-hidden="true"></span></label>
    </header>
    <p v-if="status && !status.supported" class="notice">{{ t("当前平台尚未支持自动监听；现有手动贴图不受影响。") }}</p>
    <p v-if="error || status?.error" class="notice error" role="alert">{{ te(error || status?.error) }}</p>
    <p v-if="message" role="status">{{ t(message) }}</p>
    <form class="history-search" role="search" @submit.prevent="applyFilter">
      <Search :size="16" aria-hidden="true" /><input ref="searchInput" v-model="keyword" :aria-label='t("搜索文本或文件路径")' :placeholder='t("搜索文本或文件路径（Ctrl+F）")' maxlength="200" /><button v-if="keyword" type="button" :aria-label='t("清除搜索")' @click="keyword = ''; focusSearch()"><X :size="16" /></button>
    </form>
    <div class="history-viewbar">
    <nav class="history-categories" :aria-label='t("历史分类")'>
      <button v-for="tab in categories" :key="tab.value" :class="{ active: category === tab.value }" :aria-pressed="category === tab.value" :disabled="busy || !desktop" @click="category = tab.value"><component :is="tab.icon" :size="16" aria-hidden="true" />{{ t(tab.label) }}</button>
      <PopoverRoot v-model:open="helpOpen">
        <PopoverTrigger class="history-help-trigger" :aria-label='t("帮助")' :title='t("帮助")'>?</PopoverTrigger>
        <PopoverPortal>
          <PopoverContent class="history-help-popup" side="bottom" align="end" :side-offset="6" :collision-padding="8" :aria-label='t("帮助")' @keydown.stop="helpKey">
            <p>{{ t("单击选中 · Ctrl 多选 · Shift 连选 · 双击文字或图片预览 · Ctrl+C 复制") }}</p>
            <p v-if="category === 'file'">{{ t("仅记录文件路径，原文件移动或删除后失效。") }}</p>
          </PopoverContent>
        </PopoverPortal>
      </PopoverRoot>
    </nav>
    <div class="history-layout-switch" role="group" :aria-label="t('显示方式')">
      <button type="button" :aria-pressed="layout === 'list'" :aria-label="t('列表视图')" :title="t('列表视图')" @click="setLayout('list')"><List :size="16" /></button>
      <button type="button" :aria-pressed="layout === 'grid'" :aria-label="t('网格视图')" :title="t('网格视图')" @click="setLayout('grid')"><LayoutGrid :size="16" /></button>
    </div>
    </div>
    <div v-if="confirm" class="notice history-danger-notice" role="alertdialog" :aria-label='t("确认删除历史")'>
      <p>{{ confirm === 'clear' ? t("清空历史记录？默认保留收藏。") : t("删除选中的 {0} 条记录（包含选中的收藏）？", { 0: selected.length }) }} {{ t("此操作不可撤销。") }}</p>
      <label v-if="confirm === 'clear'"><input v-model="includeFavorites" type="checkbox" />{{ t("同时删除收藏") }}</label>
      <Button variant="destructive" :disabled="busy" @click="remove">{{ t("确认删除") }}</Button><Button variant="outline" :disabled="busy" @click="confirm = null">{{ t("取消") }}</Button>
    </div>
    <DialogPortal>
    <DialogOverlay class="history-export-overlay" />
    <DialogContent class="history-export-dialog" @keydown.stop="exportKey" @escape-key-down="guardExportDismiss" @interact-outside="guardExportDismiss">
      <DialogTitle class="history-export-title">{{ t("导出") }} {{ selected.length }} {{ t("个素材") }}</DialogTitle>
      <DialogDescription class="history-export-description">{{ t("按选择顺序导出，图片使用原图。") }}</DialogDescription>
      <fieldset :disabled="busy">
      <div class="history-tools"><SelectControl v-model="options.mode" :aria-label="t('导出方式')" :options="[{ value: 'pdf', label: t('多页 PDF') }, { value: 'longPng', label: t('纵向长图 PNG') }, { value: 'separate', label: t('分别导出 PNG / TXT') }]" /><SelectControl v-if="options.mode === 'pdf'" v-model="options.pageSize" :aria-label="t('页面大小')" :options="[{ value: 'A4', label: 'A4' }, { value: 'original', label: t('图片原始比例') }]" /><label v-if="options.mode === 'pdf'">{{ t("边距 mm") }}<input v-model.number="options.marginMm" type="number" min="0" max="30" /></label><label v-if="options.mode === 'longPng'">{{ t("间距 px") }}<input v-model.number="options.gapPx" type="number" min="0" max="200" /></label><SelectControl v-if="options.mode === 'longPng'" v-model="options.background" :aria-label="t('背景')" :options="[{ value: 'white', label: t('白底') }, { value: 'transparent', label: t('透明背景') }]" /></div>
      </fieldset>
      <p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
      <p v-else-if="message" role="status">{{ t(message) }}</p>
      <div class="history-export-actions"><Button :disabled="busy || !selected.length" @click="exportItems">{{ busy ? t("正在导出…") : t("选择位置并导出") }}</Button><Button variant="outline" :disabled="busy" @click="exportOpen = false">{{ t("关闭") }}</Button></div>
    </DialogContent>
    </DialogPortal>
    <div class="history-browser">
    <VirtualClipboardList ref="list" :items="items" :layout="layout" role="region" :aria-label='t("历史记录列表")' :aria-busy="foregroundLoading">
      <template #default="{ item }">
      <ClipboardItem :item="item" :selected="selected.includes(item.id)" :busy="busy" @select="selectItem(item, $event)" @preview="previewItem = item" @restore="run(() => api.clipboardRestore(item.id), '已复制。')" @delete="selected = [item.id]; selectedItems = { [item.id]: item }; confirm = 'delete'" @pin="run(() => api.clipboardPin(item.id))" @favorite="run(async () => { await api.clipboardFavorite(item.id, !item.favorite); await refresh() })" />
      </template>
      <template #footer>
      <p v-if="!items.length" class="history-empty">{{ foregroundLoading ? t("正在加载…") : keyword || category ? t("没有匹配的记录，试试其他关键词或分类。") : status?.enabled ? t("暂无记录，复制文字或图片后会显示在这里。") : t("暂无记录，开启右上方开关后开始记录。") }}</p>
      <div v-if="cursor" class="history-load-more">
        <Button variant="ghost" :disabled="foregroundLoading || busy" :aria-label="loadingMore ? t('加载中…') : t('加载更多')" :title="t('加载更多')" @click="refresh(true)"><LoaderCircle v-if="loadingMore" :size="18" class="history-spinner" /><Ellipsis v-else :size="22" /></Button>
      </div>
      </template>
    </VirtualClipboardList>
    <!-- 侧栏独立于列表滚动并始终预留宽度，选择条目不会挤动列表或影响双击。 -->
    <aside class="history-action-rail" :aria-label='t("所选条目操作")'>
      <template v-if="selected.length">
        <span class="history-selected-count" role="status" :title="t('已选 {0} / 100 条', { 0: selected.length })">{{ t("已选") }}<br />{{ selected.length }}</span>
        <DialogTrigger as-child><Button class="history-action" :disabled="busy || !canExport" :title="canExport ? t('组合导出') : t('组合导出仅支持文本和图片，文件引用请复制后粘贴')" :aria-label='t("组合导出所选条目")' @click="error = ''; message = ''"><Download :size="18" /></Button></DialogTrigger>
        <Button class="history-action" variant="outline" :disabled="busy || !selected.some(id => ['image', 'text'].includes(selectedItems[id]?.kind))" :title='t("悬浮贴图")' :aria-label='t("将所选文字和图片贴图")' @click="pinSelected"><Pin :size="18" /></Button>
        <Button class="history-action history-delete" variant="outline" :disabled="busy" :title='t("删除所选")' :aria-label='t("删除所选条目")' @click="confirm = 'delete'"><Trash2 :size="18" /></Button>
        <Button class="history-action" variant="ghost" :disabled="busy" :title='t("取消选择")' :aria-label='t("取消选择")' @click="selected = []; selectedItems = {}"><X :size="18" /></Button>
      </template>
    </aside>
    </div>
  </section>
  </DialogRoot>
  <ClipboardPreview :item="previewItem" @close="closePreview" />
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
.history-help-popup { z-index:111; max-width:calc(100vw - 16px); padding:10px 12px; border:1px solid #d8dee4; border-radius:6px; background:#fff; color:#57606a; box-shadow:0 4px 16px #0002; font-size:12px; line-height:1.6; }
.history-help-popup p { margin:0; }
.history-help-popup p + p { margin-top:4px; }
[data-theme=dark] .history-help-popup { background:#20252c; color:#a8b3c2; border-color:#39424e; }
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
.history-image-placeholder { display:flex; height:96px; align-items:center; justify-content:center; }
.history-content img { height:96px; display:block; margin:0 auto; box-shadow:0 1px 5px #0002; }
.history-search { display:flex; align-items:center; gap:8px; color:#71717a; padding:2px 8px; border:1px solid #d8dee4; border-radius:4px; }
.history-search > svg { flex-shrink:0; }
.history-search button { display:flex; align-items:center; justify-content:center; min-width:24px; min-height:24px; }
.clipboard-history .history-search input { border:0; outline:none; max-width:none; min-width:0; width:100%; background:transparent; }
.history-search:focus-within { box-shadow:0 1px #2563eb; }
.history-categories { display:flex; flex:1; min-width:0; gap:8px; overflow-x:auto; }
.history-categories button { white-space:nowrap; flex-shrink:0; display:flex; gap:8px; align-items:center; justify-content:center; padding:10px 20px; border-bottom:2px solid transparent; color:#71717a; }
.history-categories button.active { color:#2563eb; border-bottom-color:#2563eb; background:#eff6ff; }
@media (max-width:600px) { .history-categories { gap:0; }.history-categories button { flex:1; padding:10px 8px; } }
.history-categories .history-help-trigger { flex:0 0 32px; align-self:center; height:32px; margin-left:auto; padding:0; border:0; border-radius:4px; font-size:16px; }
.history-categories .history-help-trigger:hover { background:#f6f7f8; }
[data-theme=dark] .history-categories .history-help-trigger:hover { background:#2b333d; }
.history-categories button:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.clipboard-history pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:360px; overflow:auto; }
.history-viewbar { display:flex; align-items:center; gap:8px; border-bottom:1px solid var(--desk-border, #e1e4e8); }
.history-layout-switch { display:flex; flex-shrink:0; gap:2px; padding:2px; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; }
.history-layout-switch button { display:flex; align-items:center; justify-content:center; width:32px; height:28px; border-radius:4px; color:var(--desk-muted, #57606a); }
.history-layout-switch button:hover { background:var(--desk-soft, #f6f7f8); }
.history-layout-switch button[aria-pressed=true] { color:var(--desk-accent, #2563eb); background:var(--desk-selected, #eff6ff); }
.history-load-more { display:flex; justify-content:center; padding:12px 0; }
.history-load-more button { min-width:64px; min-height:32px; }
.history-list.is-grid .history-item { position:relative; align-items:flex-start; height:216px; padding:12px; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; background:var(--desk-bg, #fff); }
.history-list.is-grid .history-item.is-selected { border-color:#93b4ff; background:var(--desk-selected, #eff6ff); }
.history-list.is-grid .history-content { display:flex; flex-direction:column; align-self:stretch; }
.history-list.is-grid .history-content img, .history-list.is-grid .history-image-placeholder { width:100%; height:140px; max-height:140px; box-shadow:none; }
.history-list.is-grid .history-text { -webkit-line-clamp:6; }
.history-list.is-grid .history-meta { width:100%; margin-top:auto; padding-top:8px; font-size:11px; }
/* 入口用红色文字，实际删除确认使用实色危险按钮；取消保持普通操作样式。 */
.clipboard-history { --history-danger:#b42318; --history-danger-bg:#fff1f0; --history-danger-border:#fecaca; }
.clipboard-history .history-clear, .clipboard-history .history-delete { color:var(--history-danger); }
.clipboard-history .history-clear:hover:enabled, .clipboard-history .history-delete:hover:enabled { color:var(--history-danger); background:var(--history-danger-bg); }
.clipboard-history .history-delete { border-color:var(--history-danger-border); }
.clipboard-history .history-danger-notice { color:var(--history-danger); border-color:var(--history-danger-border); background:var(--history-danger-bg); }
[data-theme=dark] .clipboard-history { --history-danger:#ff9c95; --history-danger-bg:#3b2327; --history-danger-border:#74414a; }
</style>
