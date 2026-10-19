<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { ContextMenuRoot, ContextMenuTrigger, ContextMenuPortal, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from 'reka-ui'
import { Copy, Star, Pin, Trash2, Files } from 'lucide-vue-next'
import { api } from '@/lib/api'
import type { ClipboardItem } from '@/lib/clipboard'
const props = defineProps<{ item: ClipboardItem; selected: boolean; busy: boolean }>()
const emit = defineEmits<{ select: []; restore: []; favorite: []; pin: []; delete: [] }>()
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
onMounted(() => {
  if (props.item.kind !== 'image' || !root.value) return
  // 仅请求进入视口的低分辨率图，组件卸载后释放 data URL 字符串。
  observer = new IntersectionObserver(entries => {
    if (!entries.some(entry => entry.isIntersecting)) return
    observer?.disconnect()
    void api.clipboardThumbnail(props.item.id).then(value => { if (!disposed) thumbnail.value = value }).catch(e => { if (!disposed) error.value = String(e) })
  })
  observer.observe(root.value)
})
onUnmounted(() => { disposed = true; observer?.disconnect() })
</script>
<template>
  <ContextMenuRoot>
    <ContextMenuTrigger as-child :disabled="busy">
      <article ref="root" class="history-item" :class="{ 'is-selected': selected }" tabindex="0" :aria-label="`${item.kind === 'image' ? '图片' : item.preview.slice(0, 80)}，回车复制，空格选择，右键操作`" @keydown="key" @click.self="select">
        <input type="checkbox" :checked="selected" :disabled="busy" :aria-label="`选择条目 ${item.id}`" @change="$emit('select')" />
        <button class="history-content" :disabled="busy" :aria-pressed="selected" :aria-label="`选择条目 ${item.id}，双击复制`" @click="select" @dblclick="emit('restore')">
          <img v-if="thumbnail" :src="thumbnail" alt="历史图片缩略图" />
          <span v-else-if="item.kind === 'image'">{{ error || `图片 ${item.width} × ${item.height}` }}</span>
          <span v-else-if="item.kind === 'file'" class="history-files"><span class="history-file-heading"><Files :size="16" />{{ item.filePaths?.length ?? 0 }} 个文件／文件夹 · 路径引用</span><span v-for="path in item.filePaths?.slice(0, 3)" :key="path" class="history-file-path" :title="path">{{ path }}</span><span v-if="(item.filePaths?.length ?? 0) > 3">另有 {{ item.filePaths!.length - 3 }} 个路径</span></span>
          <span v-else class="history-text">{{ item.preview }}</span>
          <span class="history-meta"><span>{{ new Date(item.lastSeenAt).toLocaleString() }} · {{ item.seenCount }} 次</span><span>{{ item.favorite ? '★ ' : '' }}{{ item.kind === 'image' ? `${item.width} × ${item.height}` : item.kind === 'file' ? '未备份源文件' : `${item.byteSize} B` }}</span></span>
        </button>
      </article>
    </ContextMenuTrigger>
    <!-- 复用菜单组件处理定位、边缘避让、点击外部关闭及键盘导航。 -->
    <ContextMenuPortal>
      <ContextMenuContent class="history-context-menu" :collision-padding="8" aria-label="历史条目操作">
        <ContextMenuItem :disabled="busy" @select="$emit('restore')"><Copy :size="15" />复制</ContextMenuItem>
        <ContextMenuItem v-if="item.kind === 'image'" :disabled="busy" @select="$emit('pin')"><Pin :size="15" />悬浮贴图</ContextMenuItem>
        <ContextMenuSeparator class="history-menu-separator" />
        <ContextMenuItem :disabled="busy" @select="$emit('favorite')"><Star :size="15" />{{ item.favorite ? '取消收藏' : '收藏' }}</ContextMenuItem>
        <ContextMenuItem :disabled="busy" @select="$emit('delete')"><Trash2 :size="15" />删除</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
<style>
.history-files { display:block; }
.history-file-heading { display:flex; gap:6px; align-items:center; margin-bottom:4px; }
.history-file-path { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:#57606a; font-size:12px; }
.history-context-menu { z-index:100; min-width:150px; padding:5px; background:white; border:1px solid #e4e4e7; border-radius:6px; box-shadow:0 4px 16px #0002; }
.history-context-menu [role=menuitem] { display:flex; align-items:center; gap:12px; padding:9px 12px; font-size:13px; border-radius:3px; outline:none; cursor:default; }
.history-context-menu [data-highlighted] { background:#eff6ff; color:#2563eb; }
.history-context-menu [data-disabled] { opacity:.45; pointer-events:none; }
.history-menu-separator { height:1px; background:#e4e4e7; margin:4px; }
</style>
