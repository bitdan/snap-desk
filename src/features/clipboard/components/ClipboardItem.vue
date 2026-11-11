<script setup lang="ts">
import { t, te, localizedDate } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from 'reka-ui'
import { Copy, Eye, Star, Pin, Trash2, File, FileText, FileImage, FileArchive, FileCode, FileSpreadsheet, FileVideo, FileAudio, AppWindow, Folder, Type, Link, Palette, Image, Files, PanelTop } from 'lucide-vue-next'
import { clipboardThumbnails } from '@/features/clipboard/clipboard-thumbnails'
import { clipboardColor } from '@/features/clipboard/clipboard-colors'
import type { ClipboardItem, ClipboardSelectionGesture } from '@/features/clipboard/clipboard'
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
/** shelfKept：null 表示暂存架未开启，不显示联动入口；布尔值表示条目是否已固定在架上。 */
const props = defineProps<{ item: ClipboardItem; selected: boolean; busy: boolean; shelfKept?: boolean | null }>()
const emit = defineEmits<{ select: [gesture?: ClipboardSelectionGesture]; activate: []; preview: []; restore: []; favorite: []; pin: []; delete: []; shelf: [] }>()
const menuOpen = ref(false)
useEscapeLayer(() => menuOpen.value, () => { menuOpen.value = false })
function menuKey(event: KeyboardEvent) { handleEscape(event, () => { menuOpen.value = false }) }
// 根据扩展名提供类型图标，不读取源文件；没有扩展名时用通用图标，避免把文件误判为目录。
const fileTypes = [
  { extensions: ['txt', 'md', 'log', 'rtf', 'doc', 'docx', 'odt', 'pdf'], icon: FileText, color: '#57606a' },
  { extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg', 'ico', 'heic', 'tif', 'tiff'], icon: FileImage, color: '#2563eb' },
  { extensions: ['zip', '7z', 'rar', 'tar', 'gz', 'bz2', 'xz'], icon: FileArchive, color: '#a16207' },
  { extensions: ['exe', 'msi', 'msix', 'appx'], icon: AppWindow, color: '#7c3aed' },
  { extensions: ['json', 'xml', 'yml', 'yaml', 'toml', 'js', 'ts', 'vue', 'html', 'css', 'py', 'rs', 'java', 'c', 'cpp', 'h', 'dll', 'sh', 'bat', 'ps1'], icon: FileCode, color: '#526b88' },
  { extensions: ['xls', 'xlsx', 'csv', 'ods'], icon: FileSpreadsheet, color: '#15803d' },
  { extensions: ['mp4', 'mkv', 'avi', 'mov', 'webm'], icon: FileVideo, color: '#9333ea' },
  { extensions: ['mp3', 'wav', 'flac', 'ogg', 'm4a'], icon: FileAudio, color: '#c2410c' },
]
const visibleFiles = computed(() => (props.item.filePaths ?? []).slice(0, 4).map(path => {
  const name = path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path
  const extension = name.includes('.') ? name.split('.').pop()!.toLowerCase() : ''
  const type = fileTypes.find(type => type.extensions.includes(extension))
  return { path, name, icon: /[\\/]$/.test(path) ? Folder : type?.icon ?? File, color: type?.color ?? '#71717a' }
}))
const previewable = computed(() => props.item.kind === 'image' || props.item.kind === 'text')
/** 色值与链接只影响卡片标题和色块；筛选分类仍以原生端的判定为准。 */
const color = computed(() => props.item.kind === 'text' ? clipboardColor(props.item.preview ?? '') : null)
const type = computed(() => props.item.kind === 'image' ? { icon: Image, label: '图片' }
  : props.item.kind === 'file' ? { icon: Files, label: '文件' }
  : color.value ? { icon: Palette, label: '颜色' }
  : /^https?:\/\/\S+$/i.test((props.item.preview ?? '').trim()) ? { icon: Link, label: '链接' }
  : { icon: Type, label: '文本' })
function size(bytes: number) {
  return bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`
}
const info = computed(() => props.item.kind === 'image' ? `${props.item.width} × ${props.item.height}`
  : props.item.kind === 'file' ? t('{0} 项 · 文件引用', { 0: props.item.filePaths?.length ?? 0 })
  : size(props.item.byteSize ?? 0))
/** 当天只显示时刻，更早的记录带日期；完整时间放在悬停提示。 */
const time = computed(() => {
  const date = new Date(props.item.lastSeenAt), now = new Date()
  const pad = (value: number) => String(value).padStart(2, '0')
  const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`
  return date.toDateString() === now.toDateString() ? clock : `${date.getMonth() + 1}/${date.getDate()} ${clock}`
})
/** 回车/双击复制并收起窗口，Ctrl+C 只复制；空格保留批量选择。 */
function key(event: KeyboardEvent) {
  if (props.busy || event.repeat || event.isComposing || (event.target as HTMLElement).matches('button')) return
  if (event.key === 'Enter') {
    event.preventDefault(); event.stopPropagation(); emit('activate')
  } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
    event.preventDefault(); event.stopPropagation(); emit('restore')
  } else if (event.key === ' ') {
    event.preventDefault(); emit('select', { toggle: true })
  }
}
const root = ref<HTMLElement>(); const thumbnail = ref(''); const error = ref('')
/** 双击的第二次 click 不再切换选择，避免刚选中的条目立刻被取消。 */
function select(event: MouseEvent) {
  if (props.busy) return
  root.value?.focus({ preventScroll: true })
  if (event.detail < 2) emit('select', { toggle: event.ctrlKey || event.metaKey, range: event.shiftKey })
}
function activate() { if (!props.busy) emit('activate') }
let observer: IntersectionObserver | undefined; let disposed = false
let request: AbortController | undefined
onMounted(() => {
  if (props.item.kind !== 'image' || !root.value) return
  // 离开可见区域即释放组件持有的字符串并取消排队请求；LRU 只保留有限的最近素材。
  observer = new IntersectionObserver(entries => {
    const visible = entries.at(-1)?.isIntersecting
    if (!visible) { request?.abort(); request = undefined; thumbnail.value = ''; return }
    if (thumbnail.value || request) return
    const current = new AbortController(); request = current; error.value = ''
    void clipboardThumbnails.get(props.item.id, current.signal)
      .then(value => { if (!disposed && !current.signal.aborted) thumbnail.value = value })
      .catch(e => { if (!disposed && !current.signal.aborted) error.value = String(e) })
      .finally(() => { if (request === current) request = undefined })
  }, { root: root.value.closest('.history-list'), rootMargin: '160px 0px' })
  observer.observe(root.value)
})
onUnmounted(() => { disposed = true; request?.abort(); observer?.disconnect(); thumbnail.value = '' })
</script>
<template>
  <ContextMenuRoot v-model:open="menuOpen">
    <ContextMenuTrigger as-child :disabled="busy">
      <article ref="root" class="history-item" :class="{ 'is-selected': selected }" tabindex="0" :aria-label="t('{0}，回车复制，空格选择，右键操作', { 0: item.kind === 'image' ? t('图片') : (item.preview ?? '').slice(0, 80) })" @keydown="key" @click="select" @dblclick="activate">
        <header class="history-item-head">
          <component :is="type.icon" :size="14" aria-hidden="true" />
          <span class="history-item-kind">{{ t(type.label) }}</span>
          <span class="history-item-info">{{ info }}<template v-if="item.seenCount > 1"> · {{ item.seenCount }} {{ t("次") }}</template></span>
          <Star v-if="item.favorite" class="history-item-star" :size="13" :aria-label="t('收藏')" />
          <time class="history-item-time" :title="localizedDate(item.lastSeenAt)">{{ time }}</time>
          <!-- 操作只在悬停、聚焦或选中时浮出，不为每条记录常驻一排按钮。 -->
          <span class="history-item-actions" @click.stop @dblclick.stop>
            <button type="button" :disabled="busy" :title="t('复制')" :aria-label="t('复制')" @click="emit('restore')"><Copy :size="14" /></button>
            <button v-if="previewable" type="button" :disabled="busy" :title="t('预览')" :aria-label="t('预览')" @click="emit('preview')"><Eye :size="14" /></button>
            <button type="button" :disabled="busy" :title="t('悬浮贴图')" :aria-label="t('悬浮贴图')" @click="emit('pin')"><Pin :size="14" /></button>
            <button type="button" :disabled="busy" :class="{ 'is-on': item.favorite }" :title="item.favorite ? t('取消收藏') : t('收藏')" :aria-label="item.favorite ? t('取消收藏') : t('收藏')" :aria-pressed="item.favorite" @click="emit('favorite')"><Star :size="14" /></button>
            <button type="button" class="history-item-delete" :disabled="busy" :title="t('删除')" :aria-label="t('删除')" @click="emit('delete')"><Trash2 :size="14" /></button>
          </span>
        </header>
        <div class="history-content">
          <img v-if="thumbnail" :src="thumbnail" :alt='t("历史图片缩略图")' draggable="false" />
          <span v-else-if="item.kind === 'image'" class="history-image-placeholder">{{ te(error) || t("图片 {0} × {1}", { 0: item.width, 1: item.height }) }}</span>
          <span v-else-if="item.kind === 'file'" class="history-files">
            <span v-for="(file, index) in visibleFiles" :key="index" class="history-file-row" :title="file.path"><component :is="file.icon" :size="16" :style="{ color: file.color }" aria-hidden="true" /><span class="history-file-name">{{ file.name }}</span></span>
            <span v-if="(item.filePaths?.length ?? 0) > visibleFiles.length" class="history-file-more">{{ t("另有") }} {{ (item.filePaths?.length ?? 0) - visibleFiles.length }} {{ t("项") }}</span>
          </span>
          <span v-else class="history-text"><span v-if="color" class="history-color" :style="{ background: color }" aria-hidden="true" />{{ item.preview }}</span>
        </div>
      </article>
    </ContextMenuTrigger>
    <!-- 复用菜单组件处理定位、边缘避让、点击外部关闭及键盘导航。 -->
    <ContextMenuPortal>
      <ContextMenuContent class="history-context-menu" :collision-padding="8" :aria-label='t("历史条目操作")' @keydown.stop="menuKey">
        <ContextMenuItem :disabled="busy" @select="$emit('restore')"><Copy :size="15" />{{ t("复制") }}</ContextMenuItem>
        <ContextMenuItem v-if="previewable" :disabled="busy" @select="$emit('preview')"><Eye :size="15" />{{ t("预览") }}</ContextMenuItem>
        <ContextMenuItem :disabled="busy" @select="$emit('pin')"><Pin :size="15" />{{ t("悬浮贴图") }}</ContextMenuItem>
        <ContextMenuItem v-if="shelfKept != null" :disabled="busy" @select="$emit('shelf')"><PanelTop :size="15" />{{ shelfKept ? t("从暂存架取下") : t("放到暂存架") }}</ContextMenuItem>
        <ContextMenuSeparator class="history-menu-separator" />
        <ContextMenuItem :disabled="busy" @select="$emit('favorite')"><Star :size="15" />{{ item.favorite ? t("取消收藏") : t("收藏") }}</ContextMenuItem>
        <ContextMenuItem :disabled="busy" @select="$emit('delete')"><Trash2 :size="15" />{{ t("删除") }}</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
<style>
.history-item { position:relative; display:block; margin:0; padding:8px 10px 10px; border:1px solid transparent; border-radius:8px; background:var(--clip-card, #f6f7f8); cursor:default; transition:background .12s, border-color .12s; }
.history-item:hover { background:var(--clip-card-hover, #eef1f4); }
.history-item.is-selected { background:var(--clip-selected, #eff6ff); border-color:var(--clip-selected-border, #93b4ff); }
.history-item:focus-visible { outline:2px solid var(--clip-accent, #2563eb); outline-offset:-2px; }
.history-item-head { display:flex; align-items:center; gap:6px; min-height:22px; margin-bottom:4px; color:var(--clip-muted, #6b7280); font-size:12px; line-height:1; white-space:nowrap; }
.history-item-head > svg { flex-shrink:0; }
.history-item-kind { color:var(--clip-text, #24292f); font-weight:600; }
.history-item-info { min-width:0; overflow:hidden; text-overflow:ellipsis; }
.history-item-star { flex-shrink:0; color:#d97706; fill:#f59e0b; }
.history-item-time { margin-left:auto; flex-shrink:0; font-variant-numeric:tabular-nums; }
/* 浮在标题行右侧并盖住时间，不改变卡片高度，虚拟列表无需重新测量。 */
.history-item-actions { position:absolute; top:5px; right:6px; display:none; gap:2px; padding-left:16px; background:linear-gradient(90deg, transparent, var(--clip-actions, #eef1f4) 14px); }
.history-item:hover .history-item-actions, .history-item:focus-within .history-item-actions, .history-item.is-selected .history-item-actions { display:flex; }
.history-item.is-selected .history-item-actions { background:linear-gradient(90deg, transparent, var(--clip-selected, #eff6ff) 14px); }
.history-item-actions button { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:4px; color:var(--clip-muted, #6b7280); }
.history-item-actions button:hover:enabled { background:var(--clip-press, #dfe5ec); color:var(--clip-text, #24292f); }
.history-item-actions button.is-on { color:#d97706; }
.history-item-actions button.is-on svg { fill:#f59e0b; }
.history-item-actions .history-item-delete:hover:enabled { background:var(--history-danger-bg, #fff1f0); color:var(--history-danger, #b42318); }
.history-item-actions button:focus-visible { outline:2px solid var(--clip-accent, #2563eb); outline-offset:-1px; }
.history-item-actions button:disabled { opacity:.45; }
.history-content { display:block; min-width:0; color:var(--clip-text, #24292f); text-align:left; }
.history-content img { display:block; max-width:100%; max-height:120px; object-fit:contain; border-radius:4px; }
.history-image-placeholder { display:flex; height:72px; align-items:center; color:var(--clip-muted, #6b7280); font-size:12px; }
.history-text { display:-webkit-box; -webkit-line-clamp:4; -webkit-box-orient:vertical; overflow:hidden; white-space:pre-wrap; overflow-wrap:anywhere; line-height:1.5; }
.history-color { display:inline-block; width:12px; height:12px; margin-right:6px; border:1px solid #0002; border-radius:3px; vertical-align:-1px; }
.history-files { display:block; }
.history-file-row { display:flex; align-items:center; gap:8px; min-height:22px; }
.history-file-row > svg { flex-shrink:0; }
.history-file-name { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.history-file-more { display:block; margin-left:24px; color:var(--clip-muted, #6b7280); font-size:12px; }
.history-context-menu { z-index:100; min-width:150px; padding:5px; background:white; border:1px solid #e4e4e7; border-radius:6px; box-shadow:0 4px 16px #0002; }
.history-context-menu [role=menuitem] { display:flex; align-items:center; gap:12px; padding:9px 12px; font-size:13px; border-radius:3px; outline:none; cursor:default; }
.history-context-menu [data-highlighted] { background:#eff6ff; color:#2563eb; }
.history-context-menu [data-disabled] { opacity:.45; pointer-events:none; }
.history-menu-separator { height:1px; background:#e4e4e7; margin:4px; }
[data-theme=dark] .history-context-menu [data-highlighted] { background:#223c62; color:#a9ccff; }
[data-theme=dark] .history-menu-separator { background:#39424e; }
</style>
