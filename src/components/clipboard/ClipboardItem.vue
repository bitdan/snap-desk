<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/lib/api'
import type { ClipboardItem } from '@/lib/clipboard'
const props = defineProps<{ item: ClipboardItem; selected: boolean; busy: boolean }>()
defineEmits<{ select: []; preview: []; restore: []; favorite: []; pin: [] }>()
const root = ref<HTMLElement>(); const thumbnail = ref(''); const error = ref('')
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
  <article ref="root" class="history-item">
    <input type="checkbox" :checked="selected" :disabled="busy" :aria-label="`选择条目 ${item.id}`" @change="$emit('select')" />
    <button class="history-content" :disabled="busy" @click="$emit('preview')">
      <img v-if="thumbnail" :src="thumbnail" alt="历史图片缩略图" />
      <span v-else-if="item.kind === 'image'">{{ error || `图片 ${item.width} × ${item.height}` }}</span>
      <span v-else class="history-text">{{ item.preview }}</span>
      <small>{{ new Date(item.lastSeenAt).toLocaleString() }} · {{ item.seenCount }} 次</small>
    </button>
    <div class="history-actions">
      <button :disabled="busy" @click="$emit('restore')">复制</button>
      <button :disabled="busy" :aria-pressed="item.favorite" @click="$emit('favorite')">{{ item.favorite ? '取消收藏' : '收藏' }}</button>
      <button v-if="item.kind === 'image'" :disabled="busy" @click="$emit('pin')">贴图</button>
    </div>
  </article>
</template>
