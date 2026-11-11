<script setup lang="ts">
import { useCategoryOrder, type ClipboardCategory } from '@/features/clipboard/use-category-order'
import { t, te } from '@/shared/i18n/i18n'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import ConfirmDialog from '@/shared/ui/ConfirmDialog.vue'
import { api, desktop, onClipboardChanged, onClipboardError, onClipboardState, onClipboardOpen, onClipboardBlur } from '@/services/api'
import { selectClipboardItems, type ClipboardSelectionGesture, type ClipboardItem as Item, type ClipboardStatus, type ClipboardKind, type ClipboardTextCategory, type ClipboardExportOptions } from '@/features/clipboard/clipboard'
import ClipboardComposer from '@/features/clipboard/components/ClipboardComposer.vue'
import { Search, X, Download, Pin, PinOff, Trash2, LoaderCircle, RefreshCw, CircleHelp } from 'lucide-vue-next'
import ClipboardItem from '@/features/clipboard/components/ClipboardItem.vue'
import VirtualClipboardList from '@/features/clipboard/components/VirtualClipboardList.vue'
import ClipboardPreview from '@/features/clipboard/components/ClipboardPreview.vue'
import { clipboardThumbnails } from '@/features/clipboard/clipboard-thumbnails'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape, handleWindowEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
const items = ref<Item[]>([]), selected = ref<number[]>([]), cursor = ref<string | null>(null)
const selectedItems = ref<Record<number, Item>>({})
/** 模板逐项判断成员用 Set，避免每行都对已选数组做线性 includes。 */
const selectedIds = computed(() => new Set(selected.value))
/** 条目索引按列表派生，选择与刷新不再各自重建一次 Map。 */
const itemsById = computed(() => new Map(items.value.map(item => [item.id, item])))
const previewItem = ref<Item | null>(null)
let selectionAnchor: number | null = null
/** 固定后失焦不收起，便于对照其他窗口连续复制；偏好只影响本窗口。 */
const keepOpenKey = 'snap-desk.clipboard-keep-open.v1'
const keepOpen = ref((() => {
  try { return localStorage.getItem(keepOpenKey) === '1' } catch { return false }
})())
function toggleKeepOpen() {
  keepOpen.value = !keepOpen.value
  try { localStorage.setItem(keepOpenKey, keepOpen.value ? '1' : '0') } catch { /* 存储不可用时仍允许当前窗口切换。 */ }
}
async function closePreview() {
  const index = items.value.findIndex(item => item.id === previewItem.value?.id)
  previewItem.value = null
  await nextTick()
  if (index >= 0) await list.value?.focusIndex(index)
}
const status = ref<ClipboardStatus | null>(null)
const keyword = ref(''), category = ref<ClipboardCategory>('')
const textCategory = computed(() => ['link', 'color', 'path'].includes(category.value) ? category.value as ClipboardTextCategory : undefined)
const kind = computed(() => textCategory.value ? 'text' : category.value === 'favorites' ? undefined : category.value as ClipboardKind | '')
const favorites = computed(() => category.value === 'favorites')
const categoryOptions = [{ value: '' as const, label: '全部' }, { value: 'text' as const, label: '文本' }, { value: 'link' as const, label: '链接' }, { value: 'color' as const, label: '颜色' }, { value: 'path' as const, label: '文件路径' }, { value: 'image' as const, label: '图片' }, { value: 'file' as const, label: '文件' }, { value: 'favorites' as const, label: '收藏' }]
const canExport = computed(() => selected.value.length > 0 && selected.value.every(id => ['text', 'image'].includes(selectedItems.value[id]?.kind)))
const busy = ref(false), loading = ref(false), error = ref(''), message = ref('')
const categoryOrder = useCategoryOrder(() => busy.value || !desktop, value => { category.value = value })
const { sorting, target: sortingTarget, orderError } = categoryOrder
const categories = computed(() => categoryOrder.order.value.map(value => categoryOptions.find(tab => tab.value === value)!))
const foregroundLoading = ref(false), loadingMore = ref(false)
const loadMoreFailed = ref(false)
const updatesPending = ref(false)
const confirm = ref<'delete' | 'clear' | null>(null), includeFavorites = ref(false), exportOpen = ref(false)
const helpOpen = ref(false)
useEscapeLayer(() => exportOpen.value, () => { if (!busy.value) exportOpen.value = false })
useEscapeLayer(() => helpOpen.value, () => { helpOpen.value = false })
function helpKey(event: KeyboardEvent) { handleEscape(event, () => { helpOpen.value = false }) }
/** 事件中的条件分支放在脚本内，避免被模板编译器当成 JavaScript 表达式解析。 */
function updateExportOpen(open: boolean) {
  if (!busy.value) exportOpen.value = open
}
const searchInput = ref<HTMLInputElement | null>(null)
const list = ref<InstanceType<typeof VirtualClipboardList> | null>(null)
function focusSearch() { searchInput.value?.focus(); searchInput.value?.select() }
/** 窗口没有系统标题栏：顶栏和底栏的空白处交给原生拖动，控件自身的按下不触发。 */
function dragWindow(event: PointerEvent) {
  if (!desktop || event.button !== 0 || (event.target as HTMLElement).closest('button, input, a')) return
  event.preventDefault()
  // 拖动期间的焦点转交不能被当成“离开窗口”而收起。
  draggingUntil = Date.now() + 1500
  void api.clipboardWindow('drag').catch(e => { error.value = String(e) })
}
let draggingUntil = 0
async function hideWindow() {
  if (!desktop) return
  try { await api.clipboardWindow('hide') } catch (e) { error.value = String(e) }
}
/** 子窗口（预览、导出、确认、帮助）打开或操作进行中时，失焦和 Esc 都不能收起宿主窗口。 */
function occupied() {
  return busy.value || !!confirm.value || exportOpen.value || helpOpen.value || !!previewItem.value
}
/** 原生端已延迟复核；页面仍持有焦点（例如刚点进列表）或正在拖动时同样不收起。 */
function windowBlurred() {
  if (keepOpen.value || occupied() || Date.now() < draggingUntil || globalThis.document?.hasFocus?.()) return
  void hideWindow()
}
/** 焦点不在列表内时的 Esc 兜底：先关最上层弹层，没有弹层才收起窗口。 */
function windowKey(event: KeyboardEvent) { handleWindowEscape(event, () => { void hideWindow() }) }
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
  // Ctrl+1～9 直接取当前列表的第 N 条，不必先移动焦点；在搜索框里同样生效。
  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && /^[1-9]$/.test(event.key)) {
    const item = items.value[Number(event.key) - 1]
    if (item) { event.preventDefault(); void activate(item) }
    return
  }
  if (event.key === 'Escape') {
    // 没有选择或关键词时不拦截，交给窗口级处理收起整个窗口。
    if (selected.value.length || keyword.value) handleEscape(event, () => {
      if (selected.value.length) { selected.value = []; selectedItems.value = {}; selectionAnchor = null }
      else { keyword.value = ''; focusSearch() }
    })
    return
  }
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return
  const fromSearch = target === searchInput.value
  if (fromSearch && !['ArrowDown', 'ArrowUp'].includes(event.key)) return
  const row = target.closest<HTMLElement>('.history-item')
  if (!fromSearch && !row) return
  if (!items.value.length) return
  event.preventDefault()
  const current = row ? Number(row.closest<HTMLElement>('[data-history-index]')?.dataset.historyIndex ?? -1) : -1
  const index = event.key === 'Home' ? 0 : event.key === 'End' ? items.value.length - 1
    : current < 0 ? (event.key === 'ArrowDown' ? 0 : items.value.length - 1)
    : Math.max(0, Math.min(items.value.length - 1, current + (event.key === 'ArrowDown' ? 1 : -1)))
  void list.value?.focusIndex(index)
}
/** 分类较多时单行横向滚动；滚轮的纵向量换成横向，避免窄窗口里换行挤占列表。 */
function scrollCategories(event: WheelEvent) {
  const bar = event.currentTarget as HTMLElement
  if (bar.scrollWidth > bar.clientWidth) bar.scrollLeft += event.deltaY || event.deltaX
}
function selectItem(item: Item, gesture: ClipboardSelectionGesture = { toggle: true }) {
  selected.value = selectClipboardItems(selected.value, item.id, items.value.map(value => value.id), selectionAnchor, gesture)
  if (!gesture.range) selectionAnchor = item.id
  const available = itemsById.value
  selectedItems.value = Object.fromEntries(selected.value.map(id => [id, available.get(id) ?? selectedItems.value[id]]))
}
function clearSelection() { selected.value = []; selectedItems.value = {}; selectionAnchor = null }
let revision = 0, changeRevision = 0, disposed = false, loadedMore = false, listScrolled = false
let clipboardErrorRevision = 0, latestClipboardError: string | null = null
let refreshTimer: ReturnType<typeof setTimeout> | undefined, searchTimer: ReturnType<typeof setTimeout> | undefined
const disposers: (() => void)[] = []
/** 状态查询可能早于列表返回；请求期间的新错误或恢复事件优先于旧状态快照。 */
function applyStatus(value: ClipboardStatus, errors: number) {
  status.value = errors === clipboardErrorRevision ? value : { ...value, error: latestClipboardError }
}
/** 浏览旧记录或操作所选内容时保留现场，后台更新交给用户主动刷新。 */
function canRefreshInBackground() {
  return !loadedMore && !listScrolled && !selected.value.length && !previewItem.value
    && !busy.value && !confirm.value && !exportOpen.value && !helpOpen.value
}
function listScroll(event: Event) { listScrolled = (event.target as HTMLElement).scrollTop > 0 }
/** 滚动和视口补齐共用入口；失败后等用户重试，避免持续自动重发失败请求。 */
function loadMore(retry = false) {
  if (!cursor.value || loading.value || busy.value || searchTimer || loadMoreFailed.value && !retry) return
  loadMoreFailed.value = false; error.value = ''
  void refresh(true)
}
async function refresh(append = false, background = false) {
  if (!desktop || disposed) return
  if (append && (loading.value || busy.value || !cursor.value)) return
  if (!append && !background) {
    cursor.value = null; loadMoreFailed.value = false
    clearTimeout(searchTimer); searchTimer = undefined
    clearTimeout(refreshTimer); refreshTimer = undefined
  }
  const request = ++revision, changes = changeRevision, errors = clipboardErrorRevision
  let applied = false
  loading.value = true
  foregroundLoading.value = !background; loadingMore.value = append
  try {
    const [page, state] = await Promise.all([api.clipboardList({ keyword: keyword.value, kind: kind.value || undefined, textCategory: textCategory.value, favoritesOnly: favorites.value, limit: 40, cursor: append ? cursor.value ?? undefined : undefined }), api.clipboardStatus()])
    if (disposed || request !== revision) return
    // 请求在途期间也可能开始选择或翻页，迟到的后台结果不能覆盖新的浏览现场。
    if (background && !canRefreshInBackground()) return
    const seen = new Set(append ? items.value.map(item => item.id) : [])
    items.value = [...(append ? items.value : []), ...page.items.filter(item => !seen.has(item.id))]
    cursor.value = page.nextCursor; applyStatus(state, errors)
    loadedMore = append
    const available = itemsById.value
    selected.value = selected.value.filter(id => available.has(id))
    selectedItems.value = Object.fromEntries(selected.value.map(id => [id, available.get(id)!]))
    if (selectionAnchor !== null && !available.has(selectionAnchor)) selectionAnchor = null
    if (!append) {
      updatesPending.value = changes !== changeRevision
      if (!background) { list.value?.resetScroll(); listScrolled = false }
    }
    applied = true
  } catch (e) { if (!disposed && request === revision) { error.value = String(e); if (append) loadMoreFailed.value = true } }
  finally {
    if (request === revision) {
      loading.value = false; foregroundLoading.value = false; loadingMore.value = false
      if (applied && updatesPending.value && canRefreshInBackground()) queueBackgroundRefresh()
    }
  }
}
function queueBackgroundRefresh() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined
    // 搜索防抖拥有独立计时器；等待当前查询完成后再决定是否刷新，不循环轮询。
    if (disposed || searchTimer || loading.value || !canRefreshInBackground()) return
    void refresh(false, true)
  }, 100)
}
function scheduleRefresh() {
  changeRevision++; updatesPending.value = true
  queueBackgroundRefresh()
}
// 输入停顿后查询；分类立即生效，清除旧选择和旧请求，避免显示过期结果。
function applyFilter() {
  clearTimeout(searchTimer); searchTimer = undefined
  clearSelection(); list.value?.resetScroll(); void refresh()
}
watch(keyword, () => {
  revision++; clearTimeout(searchTimer)
  loading.value = false; foregroundLoading.value = false; loadingMore.value = false
  searchTimer = setTimeout(() => { searchTimer = undefined; applyFilter() }, 250)
})
watch(category, applyFilter)
async function run(action: () => Promise<unknown>, success = '') {
  if (busy.value) return
  busy.value = true; error.value = ''; message.value = ''
  try { await action(); if (success) message.value = success }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 回车、双击或 Ctrl+数字：默认直接粘贴到唤出前的窗口（由后端交还焦点并收起）；
 *  设置里关闭直接粘贴，或没有可粘贴的外部窗口时，只写回系统剪贴板再收起。固定时保留窗口。 */
async function activate(item: Item) {
  if (busy.value) return
  let pasted = false
  if (status.value?.pasteOnActivate) await run(async () => { pasted = await api.clipboardPaste(item.id, keepOpen.value) })
  else await run(() => api.clipboardRestore(item.id))
  if (error.value) return
  message.value = pasted ? '已粘贴。' : '已复制。'
  if (!pasted && !keepOpen.value) await hideWindow()
}
/** 收藏只更新本地一条记录，避免整页刷新丢掉滚动位置；收藏分类下仍需重查以移除条目。 */
async function toggleFavorite(item: Item) {
  await run(async () => {
    const favorite = !item.favorite
    await api.clipboardFavorite(item.id, favorite)
    items.value = items.value.map(value => value.id === item.id ? { ...value, favorite } : value)
    const picked = selectedItems.value[item.id]
    if (picked) selectedItems.value = { ...selectedItems.value, [item.id]: { ...picked, favorite } }
    if (favorites.value) await refresh()
  })
}
/** 暂存架与历史共用条目 ID；这里只读取架上有哪些条目，用于右键菜单的放上/取下。 */
const shelfIds = ref<Set<number> | null>(null)
async function loadShelf() {
  if (!desktop) return
  try {
    const snapshot = await api.shelfSnapshot()
    if (!disposed) shelfIds.value = snapshot.settings.enabled ? new Set(snapshot.items.map(item => item.id)) : null
  } catch { if (!disposed) shelfIds.value = null }
}
async function toggleShelf(item: Item) {
  const placed = shelfIds.value?.has(item.id) ?? false
  await run(async () => {
    // 固定同时撤销“已从架上移除”的隐藏标记；取下只影响架子，不删除历史。
    await api.shelfItemAction(item.id, placed ? 'remove' : 'keep')
    await loadShelf()
  }, placed ? '已从暂存架取下。' : '已放到暂存架。')
}
async function setRecording(enabled: boolean) {
  await run(async () => {
    const errors = clipboardErrorRevision
    applyStatus(await api.clipboardSetEnabled(enabled), errors)
  }, enabled ? '已开始记录，仅记录此后复制的内容。' : '')
}
/** 确认框由模态层管理焦点；删除期间禁止关闭，失败时在框内保留原因。 */
function updateConfirmOpen(open: boolean) {
  if (!open && !busy.value) confirm.value = null
}
function requestDelete(action: 'clear' | 'delete') {
  error.value = ''; includeFavorites.value = false; confirm.value = action
}
async function remove() {
  const action = confirm.value
  if (!action || busy.value) return
  await run(async () => {
    if (action === 'clear') await api.clipboardClear(includeFavorites.value)
    else await api.clipboardDelete(selected.value)
    clipboardThumbnails.clear()
    clearSelection(); confirm.value = null; await refresh()
  }, '历史条目已删除。')
}
/** 打开编辑器时按选择顺序取一份素材快照；顺序、移除与改写都在编辑器内完成。 */
const composerItems = ref<Item[]>([])
function openComposer() {
  if (!canExport.value) return
  error.value = ''; message.value = ''
  composerItems.value = selected.value.map(id => selectedItems.value[id]).filter(Boolean)
  exportOpen.value = true
}
async function exportItems(payload: ClipboardExportOptions) {
  await run(async () => {
    const result = await api.clipboardExport(payload)
    message.value = result.status === 'cancelled' ? '已取消导出。' : `已保存 ${result.outputPaths.length} 个文件。`
    if (result.status === 'saved') exportOpen.value = false
  })
}
async function pinSelected() {
  await run(async () => {
    let done = 0
    for (const id of selected.value) {
      if (!['image', 'text', 'file'].includes(selectedItems.value[id]?.kind)) continue
      try { await api.clipboardPin(id); done++ } catch (e) { throw new Error(`已创建 ${done} 张贴图；${String(e)}`) }
    }
    message.value = `已创建 ${done} 张贴图。`
  })
}
/** 恢复事件清除列表提示与已缓存状态中的旧错误，避免仍回退显示故障快照。 */
function clipboardError(value: string) {
  clipboardErrorRevision++; latestClipboardError = value || null
  error.value = value
  if (status.value) status.value.error = latestClipboardError
}
/** 窗口常驻复用：再次唤出时回到搜索框，没有进行中的选择或弹窗就回到最新记录。 */
function opened() {
  focusSearch(); message.value = ''
  void loadShelf()
  if (!selected.value.length && !occupied()) void refresh()
}
onMounted(async () => {
  window.addEventListener('keydown', windowKey)
  await nextTick(); focusSearch()
  if (!desktop) return
  try {
    for (const subscribe of [() => onClipboardOpen(opened), () => onClipboardBlur(windowBlurred), () => onClipboardChanged(scheduleRefresh), () => onClipboardError(clipboardError), () => onClipboardState(value => { status.value = value })]) {
      const off = await subscribe(); if (disposed) { off(); return }; disposers.push(off)
    }
    void loadShelf()
    await refresh()
  } catch (e) { error.value = String(e) }
})
onUnmounted(() => { disposed = true; revision++; window.removeEventListener('keydown', windowKey); clearTimeout(refreshTimer); clearTimeout(searchTimer); disposers.forEach(off => off()); clipboardThumbnails.clear() })
</script>
<template>
  <section class="clipboard-history" @keydown="navigate">
    <header class="history-topbar" @pointerdown="dragWindow">
      <form class="history-search" role="search" @submit.prevent="applyFilter">
        <Search :size="16" aria-hidden="true" /><input ref="searchInput" v-model="keyword" :aria-label='t("搜索文本或文件路径")' :placeholder='t("搜索文本或文件路径（Ctrl+F）")' maxlength="200" /><button v-if="keyword" type="button" :aria-label='t("清除搜索")' @click="keyword = ''; focusSearch()"><X :size="16" /></button>
      </form>
      <button type="button" class="history-icon-button" :aria-pressed="keepOpen" :aria-label="t('固定窗口，失去焦点时不收起')" :title="t('固定窗口，失去焦点时不收起')" @click="toggleKeepOpen"><Pin v-if="keepOpen" :size="16" /><PinOff v-else :size="16" /></button>
      <button type="button" class="history-icon-button" :aria-label="t('收起')" :title="t('收起（Esc）')" @click="hideWindow"><X :size="16" /></button>
    </header>
    <nav class="history-categories" :aria-label='t("历史分类")' @wheel.passive="scrollCategories">
      <button v-for="tab in categories" :key="tab.value" :class="{ active: category === tab.value, 'sorting-source': sorting === tab.value, 'sorting-target': sortingTarget === tab.value && sorting !== tab.value }" :data-clipboard-category="tab.value" :title="t('{0}（拖动排序，Alt+←/→调整顺序）', { 0: t(tab.label) })" :aria-pressed="category === tab.value" :disabled="busy || !desktop" @pointerdown="categoryOrder.start($event, tab.value)" @pointermove="categoryOrder.move" @pointerup="categoryOrder.finish" @pointercancel="categoryOrder.cancel" @lostpointercapture="categoryOrder.cancel" @dragstart.prevent @keydown="categoryOrder.key($event, tab.value)" @click="categoryOrder.choose($event, tab.value)">{{ t(tab.label) }}</button>
    </nav>
    <div v-if="status && status.supported && !status.enabled" class="history-notice history-paused" role="status"><span>{{ t("记录已暂停，新复制的内容不会保存。") }}</span><Button variant="ghost" :disabled="busy || !desktop" @click="setRecording(true)">{{ t("继续记录") }}</Button></div>
    <p v-if="updatesPending" class="history-notice" role="status">{{ t("内容已更新，点击刷新。") }}</p>
    <p v-if="status && !status.supported" class="history-notice">{{ t("当前平台尚未支持自动监听；现有手动贴图不受影响。") }}</p>
    <p v-if="error || status?.error" class="history-notice error" role="alert">{{ te(error || status?.error) }}</p>
    <p v-if="orderError" class="history-notice" role="status">{{ t(orderError) }}</p>
    <VirtualClipboardList ref="list" :items="items" :has-more="!!cursor && !loadMoreFailed" :loading="loading || busy" role="region" :aria-label='t("历史记录列表")' :aria-busy="foregroundLoading" @scroll="listScroll" @near-end="loadMore">
      <template #default="{ item }">
      <ClipboardItem :item="item" :selected="selectedIds.has(item.id)" :busy="busy" :shelf-kept="shelfIds ? shelfIds.has(item.id) : null" @select="selectItem(item, $event)" @activate="activate(item)" @preview="previewItem = item" @restore="run(() => api.clipboardRestore(item.id), '已复制。')" @delete="selected = [item.id]; selectedItems = { [item.id]: item }; requestDelete('delete')" @pin="run(() => api.clipboardPin(item.id))" @favorite="toggleFavorite(item)" @shelf="toggleShelf(item)" />
      </template>
      <template #footer>
      <p v-if="!items.length" class="history-empty">{{ foregroundLoading ? t("正在加载…") : keyword || category ? t("没有匹配的记录，试试其他关键词或分类。") : status?.enabled === false ? t("暂无记录。") : t("暂无记录，复制文字或图片后会显示在这里。") }}</p>
      <div v-if="loadingMore || loadMoreFailed && cursor" class="history-load-more">
        <span v-if="loadingMore" role="status"><LoaderCircle :size="18" class="history-spinner" />{{ t('加载中…') }}</span>
        <Button v-else variant="ghost" :disabled="loading || busy" @click="loadMore(true)">{{ t('重试') }}</Button>
      </div>
      </template>
    </VirtualClipboardList>
    <!-- 底栏固定高度：未选择时显示容量与全局操作，选择后原位换成批量操作，列表不被挤动。 -->
    <footer class="history-footer" @pointerdown="dragWindow">
      <span v-if="selected.length" class="history-selected-count" role="status" :title="t('已选 {0} / 100 条', { 0: selected.length })">{{ t("已选") }} {{ selected.length }}</span>
      <span v-else-if="message" class="history-message" role="status">{{ t(message) }}</span>
      <span v-else class="history-count">{{ status?.count ?? 0 }} {{ t("条 ·") }} {{ ((status?.byteSize ?? 0) / 1048576).toFixed(1) }} MiB</span>
      <div class="history-footer-actions" role="group" :aria-label='selected.length ? t("所选条目操作") : undefined'>
        <template v-if="selected.length">
          <Button class="history-action" variant="ghost" :disabled="busy || !canExport" :title="canExport ? t('组合导出') : t('组合导出仅支持文本和图片，文件引用请复制后粘贴')" :aria-label='t("组合导出所选条目")' @click="openComposer"><Download :size="16" /></Button>
          <Button class="history-action" variant="ghost" :disabled="busy" :title='t("悬浮贴图")' :aria-label='t("将所选内容贴图")' @click="pinSelected"><Pin :size="16" /></Button>
          <Button class="history-action history-delete" variant="ghost" :disabled="busy" :title='t("删除所选")' :aria-label='t("删除所选条目")' @click="requestDelete('delete')"><Trash2 :size="16" /></Button>
          <Button class="history-action" variant="ghost" :disabled="busy" :title='t("取消选择")' :aria-label='t("取消选择")' @click="clearSelection"><X :size="16" /></Button>
        </template>
        <button v-else type="button" class="history-icon-button" :aria-label='t("帮助")' :title='t("帮助")' @click="helpOpen = true"><CircleHelp :size="15" /></button>
        <!-- 刷新在两种状态下都保留：有选择时后台更新只提示，需要用户主动刷新。 -->
        <Button variant="ghost" :disabled="busy || foregroundLoading || !desktop" @click="refresh()"><RefreshCw :size="14" aria-hidden="true" />{{ t("刷新") }}</Button>
        <Button v-if="!selected.length" variant="ghost" class="history-clear" :disabled="busy || !status?.count" @click="requestDelete('clear')">{{ t("清空") }}</Button>
      </div>
    </footer>
    <DetachedWindow :open="helpOpen" :title="t('帮助')" :width="480" :height="220" @close="helpOpen = false">
      <section class="history-help-popup" :aria-label='t("帮助")' @keydown.stop="helpKey">
        <p>{{ status?.pasteOnActivate ? t("回车或双击粘贴到原窗口 · Ctrl+1～9 粘贴第 N 条 · 单击选中 · Ctrl 多选 · Shift 连选 · Ctrl+C 仅复制 · 右键更多操作") : t("回车或双击复制并收起 · Ctrl+1～9 复制第 N 条 · 单击选中 · Ctrl 多选 · Shift 连选 · Ctrl+C 仅复制 · 右键更多操作") }}</p>
        <p v-if="category === 'file'">{{ t("仅记录文件路径，原文件移动或删除后失效。") }}</p>
      </section>
    </DetachedWindow>
    <DetachedWindow :open="exportOpen" :title="t('组合导出')" :width="960" :height="640" :busy="busy" @close="updateExportOpen(false)">
      <ClipboardComposer :items="composerItems" :busy="busy" :error="error" :message="message" @export="exportItems" @close="updateExportOpen(false)" />
    </DetachedWindow>
  </section>
  <ConfirmDialog :open="!!confirm" :title="confirm === 'clear' ? t('清空') : t('删除所选')" :description="(confirm === 'clear' ? t('清空历史记录？默认保留收藏。') : t('删除选中的 {0} 条记录（包含选中的收藏）？', { 0: selected.length })) + ' ' + t('此操作不可撤销。')" :confirm-label="t('确认删除')" :busy="busy" destructive :error="te(error)" @update:open="updateConfirmOpen" @confirm="remove">
    <label v-if="confirm === 'clear'" class="history-confirm-option"><input v-model="includeFavorites" type="checkbox" :disabled="busy" />{{ t("同时删除收藏") }}</label>
  </ConfirmDialog>
  <ClipboardPreview :item="previewItem" @close="closePreview" />
</template>
<style>
/* 独立窗口自带一套色值：不依赖主窗口的 .app-shell 变量，深色主题在此统一覆盖。 */
.clipboard-history { --clip-bg:#fff; --clip-text:#24292f; --clip-muted:#6b7280; --clip-border:#e1e4e8; --clip-card:#f6f7f8; --clip-card-hover:#eef1f4; --clip-actions:#eef1f4; --clip-press:#dfe5ec; --clip-selected:#eff6ff; --clip-selected-border:#93b4ff; --clip-accent:#2563eb; --clip-notice:#eef4fc; --clip-notice-text:#315a86; --history-danger:#b42318; --history-danger-bg:#fff1f0; }
[data-theme=dark] .clipboard-history { --clip-bg:#1b1f24; --clip-text:#e6edf3; --clip-muted:#9aa7b4; --clip-border:#39424e; --clip-card:#252b33; --clip-card-hover:#2d3540; --clip-actions:#2d3540; --clip-press:#3a4452; --clip-selected:#223c62; --clip-selected-border:#6386b8; --clip-accent:#6ea8ff; --clip-notice:#22364d; --clip-notice-text:#c4dfff; --history-danger:#ff9c95; --history-danger-bg:#3b2327; color-scheme:dark; }
.clipboard-history { display:flex; flex-direction:column; height:100%; min-height:0; border:1px solid var(--clip-border); background:var(--clip-bg); color:var(--clip-text); font-size:13px; user-select:none; }
.clipboard-history > * { flex-shrink:0; }
.history-topbar { display:flex; align-items:center; gap:4px; padding:10px 10px 8px; cursor:default; }
.history-search { display:flex; flex:1; align-items:center; gap:8px; min-width:0; height:32px; padding:0 8px; border:1px solid transparent; border-radius:8px; background:var(--clip-card); color:var(--clip-muted); }
.history-search > svg { flex-shrink:0; }
.history-search input { flex:1; min-width:0; height:100%; border:0; outline:none; background:transparent; color:var(--clip-text); user-select:text; }
.history-search input::placeholder { color:var(--clip-muted); }
.history-search button { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:4px; }
.history-search:focus-within { border-color:var(--clip-accent); background:var(--clip-bg); }
.history-icon-button { display:flex; flex-shrink:0; align-items:center; justify-content:center; width:32px; height:32px; border-radius:6px; color:var(--clip-muted); }
.history-icon-button:hover { background:var(--clip-card-hover); color:var(--clip-text); }
.history-icon-button[aria-pressed=true] { background:var(--clip-selected); color:var(--clip-accent); }
.history-icon-button:focus-visible, .history-search button:focus-visible { outline:2px solid var(--clip-accent); outline-offset:-2px; }
.history-categories { display:flex; gap:4px; padding:0 10px 8px; overflow-x:auto; scrollbar-width:none; }
.history-categories::-webkit-scrollbar { display:none; }
.history-categories button { flex-shrink:0; height:26px; padding:0 9px; border-radius:13px; color:var(--clip-muted); font-size:12px; white-space:nowrap; touch-action:none; }
.history-categories button:hover:enabled { background:var(--clip-card-hover); color:var(--clip-text); }
.history-categories button.active { background:var(--clip-selected); color:var(--clip-accent); font-weight:600; }
.history-categories button.sorting-source { opacity:.5; }
.history-categories button.sorting-target { box-shadow:inset 2px 0 var(--clip-accent); }
.history-categories button:focus-visible { outline:2px solid var(--clip-accent); outline-offset:-2px; }
.history-notice { display:flex; align-items:center; justify-content:space-between; gap:8px; margin:0 10px 8px; padding:6px 10px; border-radius:6px; background:var(--clip-notice, #eef4fc); color:var(--clip-notice-text, #315a86); font-size:12px; line-height:1.5; overflow-wrap:anywhere; user-select:text; }
.history-notice.error { background:var(--history-danger-bg, #fff1f0); color:var(--history-danger, #b42318); }
.history-paused button { flex-shrink:0; height:24px; padding:0 8px; font-size:12px; }
.clipboard-history .history-list { flex:1 1 0; min-height:0; padding:0 10px; overflow-y:auto; overscroll-behavior:contain; scrollbar-width:thin; }
.history-list .history-virtual-row { padding-bottom:6px; }
.history-empty { padding:48px 16px; text-align:center; color:var(--clip-muted); }
.history-load-more { display:flex; justify-content:center; padding:8px 0 12px; }
.history-load-more > span { display:inline-flex; align-items:center; gap:6px; }
.history-load-more button { min-width:64px; min-height:32px; }
.history-spinner { animation:history-spin 1s linear infinite; }
@keyframes history-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion:reduce) { .history-spinner { animation:none; } }
.history-footer { display:flex; align-items:center; justify-content:space-between; gap:8px; height:40px; padding:0 6px 0 12px; border-top:1px solid var(--clip-border); color:var(--clip-muted); font-size:12px; }
.history-count, .history-message, .history-selected-count { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.history-message { color:var(--ui-success-fg, #24834b); }
.history-selected-count { color:var(--clip-accent); font-weight:600; }
.history-footer-actions { display:flex; flex-shrink:0; align-items:center; gap:2px; }
.history-footer-actions button { height:28px; padding:0 8px; gap:4px; font-size:12px; }
.history-footer-actions .history-icon-button, .history-footer-actions .history-action { width:28px; padding:0; }
.clipboard-history .history-clear, .clipboard-history .history-delete { color:var(--history-danger); }
.clipboard-history .history-clear:hover:enabled, .clipboard-history .history-delete:hover:enabled { color:var(--history-danger); background:var(--history-danger-bg); }
/* 帮助、导出与确认内容被移入独立子窗口，样式不能依赖 .clipboard-history 祖先。 */
.history-help-popup { min-height:100%; padding:16px; background:#fff; color:#57606a; font-size:12px; line-height:1.6; }
.history-help-popup p { margin:0; }
.history-help-popup p + p { margin-top:4px; }
[data-theme=dark] .history-help-popup { background:#20252c; color:#a8b3c2; }
.history-confirm-option { display:flex; align-items:center; gap:8px; min-height:32px; }
</style>
