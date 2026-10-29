<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription } from 'reka-ui'
import { X, LoaderCircle } from 'lucide-vue-next'
import { api } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import type { ClipboardItem } from '@/features/clipboard/clipboard'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'

const props = defineProps<{ item: ClipboardItem | null }>()
const emit = defineEmits<{ close: [] }>()
const image = ref(''), text = ref(''), error = ref(''), loading = ref(false)
let revision = 0
/** 预览只持有当前条目的完整内容；关闭或切换时失效旧请求并立即释放。 */
function close() {
  revision++; image.value = ''; text.value = ''; loading.value = false
  emit('close')
}
function updateOpen(open: boolean) { if (!open) close() }
useEscapeLayer(() => !!props.item, close)
function key(event: KeyboardEvent) { handleEscape(event, close) }
async function load() {
  const request = ++revision, id = props.item?.id, kind = props.item?.kind
  image.value = ''; text.value = ''; error.value = ''; loading.value = id !== undefined
  if (id === undefined) return
  try {
    const value = kind === 'text' ? await api.clipboardText(id) : await api.clipboardImage(id)
    if (request === revision) {
      if (kind === 'text') text.value = value
      else image.value = value
    }
  } catch (e) { if (request === revision) error.value = String(e) }
  finally { if (request === revision) loading.value = false }
}
watch(() => props.item?.id, load, { immediate: true })
onUnmounted(() => { revision++; image.value = ''; text.value = '' })
</script>

<template>
  <DialogRoot :open="!!item" @update:open="updateOpen">
    <DialogPortal>
      <DialogOverlay class="history-preview-overlay" />
      <DialogContent class="history-preview-dialog" @keydown.stop="key">
        <header class="history-preview-header">
          <DialogTitle>{{ item?.kind === 'text' ? t('文字预览') : t('图片预览') }}</DialogTitle>
          <DialogDescription>{{ item?.kind === 'text' ? `${item.byteSize} B` : `${item?.width} × ${item?.height} px` }} · {{ t('Esc 关闭') }}</DialogDescription>
          <Button variant="ghost" :aria-label="t('关闭预览')" :title="t('Esc 关闭')" @click="close"><X :size="18" /></Button>
        </header>
        <div class="history-preview-body" :aria-busy="loading">
          <span v-if="loading" class="history-preview-status" role="status"><LoaderCircle :size="20" class="history-spinner" />{{ t('正在加载…') }}</span>
          <div v-else-if="error" class="history-preview-status" role="alert"><span>{{ te(error) }}</span><Button variant="outline" @click="load">{{ t('重试') }}</Button></div>
          <img v-else-if="image" :src="image" :alt="t('历史图片原图')" @error="error = '历史图片预览失败。'" />
          <textarea v-else-if="item?.kind === 'text'" class="history-preview-text" readonly :aria-label="t('历史完整文字')" :value="text" spellcheck="false" />
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style>
.history-preview-overlay { position:fixed; inset:0; z-index:120; background:#10182899; }
.history-preview-dialog { position:fixed; inset:24px; z-index:121; display:flex; flex-direction:column; max-width:1280px; max-height:960px; margin:auto; overflow:hidden; border:1px solid #d8dee4; border-radius:8px; background:#fff; color:#24292f; box-shadow:0 12px 40px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.history-preview-header { display:flex; align-items:center; gap:12px; padding:8px 12px; border-bottom:1px solid #e1e4e8; }
.history-preview-header h2 { margin:0; font-size:14px; font-weight:600; }
.history-preview-header p { margin:0; color:#57606a; font-size:12px; }
.history-preview-header button { margin-left:auto; min-width:32px; min-height:32px; padding:4px; }
.history-preview-body { display:flex; align-items:center; justify-content:center; flex:1; min-height:0; padding:16px; overflow:auto; background:#f6f7f8; }
.history-preview-body img { display:block; max-width:100%; max-height:100%; object-fit:contain; }
.history-preview-text { align-self:stretch; flex:1; min-width:0; width:100%; resize:none; padding:16px; border:1px solid #e1e4e8; border-radius:6px; background:#fff; color:#24292f; font:14px/1.7 'Segoe UI','Microsoft YaHei',sans-serif; white-space:pre-wrap; overflow-wrap:anywhere; }
.history-preview-text:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.history-preview-status { display:flex; align-items:center; justify-content:center; flex-wrap:wrap; gap:12px; }
.history-spinner { animation:history-spin 1s linear infinite; }
@keyframes history-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion:reduce) { .history-spinner { animation:none; } }
[data-theme=dark] .history-preview-dialog { background:#20252c; color:#e6edf3; border-color:#39424e; }
[data-theme=dark] .history-preview-header { border-color:#39424e; }
[data-theme=dark] .history-preview-header p { color:#a8b3c2; }
[data-theme=dark] .history-preview-body { background:#181b20; }
[data-theme=dark] .history-preview-text { background:#20252c; color:#e6edf3; border-color:#39424e; }
</style>
