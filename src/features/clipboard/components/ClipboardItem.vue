<script setup lang="ts">
import { t, te, localizedDate } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from 'reka-ui'
import { Copy, Star, Pin, Trash2, File, FileText, FileImage, FileArchive, FileCode, FileSpreadsheet, FileVideo, FileAudio, AppWindow, Folder } from 'lucide-vue-next'
import { clipboardThumbnails } from '@/features/clipboard/clipboard-thumbnails'
import type { ClipboardItem } from '@/features/clipboard/clipboard'
const props = defineProps<{ item: ClipboardItem; selected: boolean; busy: boolean }>()
const emit = defineEmits<{ select: []; restore: []; favorite: []; pin: []; delete: [] }>()
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
/** 回车与双击复用右键菜单的复制接口；空格和复选框仍负责批量选择。 */
function key(event: KeyboardEvent) {
  if (props.busy || event.repeat || event.isComposing || (event.target as HTMLElement).matches('input')) return
  if (event.key === 'Enter' || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c')) {
    event.preventDefault(); event.stopPropagation(); emit('restore')
  } else if (event.key === ' ' && event.target === event.currentTarget) {
    event.preventDefault(); emit('select')
  }
}
const root = ref<HTMLElement>(); const thumbnail = ref(''); const error = ref('')
/** 双击的第二次 click 不再切换选择，避免刚选中的条目立刻被取消。 */
function select(event: MouseEvent) {
  if (props.busy) return
  root.value?.focus({ preventScroll: true })
  if (event.detail < 2) emit('select')
}
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
  <ContextMenuRoot>
    <ContextMenuTrigger as-child :disabled="busy">
      <article ref="root" class="history-item" :class="{ 'is-selected': selected }" tabindex="0" :aria-label="t('{0}，回车复制，空格选择，右键操作', { 0: item.kind === 'image' ? t('图片') : item.preview.slice(0, 80) })" @keydown="key" @click.self="select">
        <input type="checkbox" :checked="selected" :disabled="busy" :aria-label="t('选择条目 {0}', { 0: item.id })" @change="$emit('select')" />
        <button class="history-content" :disabled="busy" :aria-pressed="selected" :aria-label="t('选择条目 {0}，双击复制', { 0: item.id })" @click="select" @dblclick="emit('restore')">
          <img v-if="thumbnail" :src="thumbnail" :alt='t("历史图片缩略图")' />
          <span v-else-if="item.kind === 'image'" class="history-image-placeholder">{{ te(error) || t("图片 {0} × {1}", { 0: item.width, 1: item.height }) }}</span>
          <span v-else-if="item.kind === 'file'" class="history-files">
            <span v-for="(file, index) in visibleFiles" :key="index" class="history-file-row" :title="file.path"><component :is="file.icon" :size="18" :style="{ color: file.color }" aria-hidden="true" /><span class="history-file-name">{{ file.name }}</span></span>
            <span v-if="(item.filePaths?.length ?? 0) > visibleFiles.length" class="history-file-more">{{ t("另有") }} {{ item.filePaths!.length - visibleFiles.length }} {{ t("项") }}</span>
          </span>
          <span v-else class="history-text">{{ item.preview }}</span>
          <span class="history-meta"><span>{{ localizedDate(item.lastSeenAt) }} · {{ item.seenCount }} {{ t("次") }}</span><span>{{ item.favorite ? '★ ' : '' }}{{ item.kind === 'image' ? `${item.width} × ${item.height}` : item.kind === 'file' ? t("{0} 项 · 文件引用", { 0: item.filePaths?.length ?? 0 }) : `${item.byteSize} B` }}</span></span>
        </button>
      </article>
    </ContextMenuTrigger>
    <!-- 复用菜单组件处理定位、边缘避让、点击外部关闭及键盘导航。 -->
    <ContextMenuPortal>
      <ContextMenuContent class="history-context-menu" :collision-padding="8" :aria-label='t("历史条目操作")'>
        <ContextMenuItem :disabled="busy" @select="$emit('restore')"><Copy :size="15" />{{ t("复制") }}</ContextMenuItem>
        <ContextMenuItem v-if="item.kind === 'image'" :disabled="busy" @select="$emit('pin')"><Pin :size="15" />{{ t("悬浮贴图") }}</ContextMenuItem>
        <ContextMenuSeparator class="history-menu-separator" />
        <ContextMenuItem :disabled="busy" @select="$emit('favorite')"><Star :size="15" />{{ item.favorite ? t("取消收藏") : t("收藏") }}</ContextMenuItem>
        <ContextMenuItem :disabled="busy" @select="$emit('delete')"><Trash2 :size="15" />{{ t("删除") }}</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
<style>
.history-files { display:block; }
.history-file-row { display:flex; align-items:center; gap:8px; min-height:24px; }
.history-file-row > svg { flex-shrink:0; }
.history-file-name { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.history-file-more { display:block; margin-left:26px; color:#71717a; font-size:12px; }
.history-context-menu { z-index:100; min-width:150px; padding:5px; background:white; border:1px solid #e4e4e7; border-radius:6px; box-shadow:0 4px 16px #0002; }
.history-context-menu [role=menuitem] { display:flex; align-items:center; gap:12px; padding:9px 12px; font-size:13px; border-radius:3px; outline:none; cursor:default; }
.history-context-menu [data-highlighted] { background:#eff6ff; color:#2563eb; }
.history-context-menu [data-disabled] { opacity:.45; pointer-events:none; }
.history-menu-separator { height:1px; background:#e4e4e7; margin:4px; }
</style>
