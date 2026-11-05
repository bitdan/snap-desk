<script setup lang="ts">
import { computed, onUnmounted, ref, shallowRef, watch } from 'vue'
import DetachedWindow from '@/shared/ui/DetachedWindow.vue'
import { X, LoaderCircle, ExternalLink, FolderOpen } from 'lucide-vue-next'
import { api } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import type { ClipboardItem } from '@/features/clipboard/clipboard'
import { clipboardColor } from '@/features/clipboard/clipboard-colors'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'

const props = defineProps<{ item: ClipboardItem | null }>()
const emit = defineEmits<{ close: [] }>()
const image = ref(''), text = ref(''), error = ref(''), loading = ref(false)
const previewImage = shallowRef<HTMLImageElement | null>(null)
const previewWindow = shallowRef<InstanceType<typeof DetachedWindow> | null>(null)
const opening = ref(false), openError = ref('')
const color = computed(() => clipboardColor(text.value))
// 与原生端路径分类一致；不在预览时访问磁盘，仅点击时请求原生定位。
const path = computed(() => {
  let value = text.value.trim()
  if (!value || /[\u0000-\u001f\u007f]/u.test(value)) return null
  if (value.startsWith('"') && value.endsWith('"')) value = value.slice(1, -1)
  const drive = /^[a-z]:[\\/]/i.test(value)
  const unc = value.startsWith('\\\\') && !/^[.?]/.test(value.slice(2)) && value.slice(2).split('\\').filter(Boolean).length >= 2
  const unix = value.startsWith('/') && !value.startsWith('//')
  return (drive || unc || unix) && !/[<>|"*?]/.test(value) ? value : null
})
// 前端只决定链接外观；真正打开时原生端重新读取原文并校验协议。
const link = computed(() => {
  const value = text.value.trim()
  if (!value || /[\s\u0000-\u001f\u007f]/u.test(value)) return null
  try {
    const url = new URL(value.startsWith('www.') ? `https://${value}` : value)
    return ['http:', 'https:', 'ftp:'].includes(url.protocol) && url.hostname ? url.href : null
  } catch { return null }
})
let revision = 0
/** 打开失败保留预览供重试，切换记录时不显示上一条的操作结果。 */
async function openTarget(kind: 'link' | 'path') {
  if (!(kind === 'link' ? link.value : path.value) || opening.value || props.item?.id === undefined) return
  const request = revision, id = props.item.id
  opening.value = true; openError.value = ''
  try { await (kind === 'link' ? api.clipboardOpenLink(id) : api.clipboardOpenPath(id)) }
  catch (e) { if (request === revision) openError.value = String(e) }
  finally { if (request === revision) opening.value = false }
}
/** 预览只持有当前条目的完整内容；关闭或切换时失效旧请求并立即释放。 */
function close() {
  // Esc 和关闭按钮直接释放独立窗口，避免只清空内容后留下空白窗。
  previewWindow.value?.close()
  revision++; image.value = ''; text.value = ''; loading.value = false; opening.value = false; openError.value = ''
  emit('close')
}
useEscapeLayer(() => !!props.item, close)
function key(event: KeyboardEvent) { handleEscape(event, close) }
/** 切换条目或重试后，已卸载图片的迟到解码错误不能覆盖当前预览。 */
function imageFailed(event: Event) {
  if (event.currentTarget === previewImage.value && image.value) error.value = '历史图片预览失败。'
}
async function load() {
  const request = ++revision, id = props.item?.id, kind = props.item?.kind
  image.value = ''; text.value = ''; error.value = ''; loading.value = id !== undefined
  opening.value = false; openError.value = ''
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
  <DetachedWindow ref="previewWindow" :open="!!item" :title="t('预览')" :width="900" :height="640" @close="close">
      <section class="history-preview-dialog" @keydown.stop="key">
        <header class="history-preview-header">
          <p>{{ item?.kind === 'text' ? `${item.byteSize} B` : `${item?.width} × ${item?.height} px` }}</p>
          <Button variant="ghost" :aria-label="t('关闭预览')" :title="t('Esc 关闭')" @click="close"><X :size="18" /></Button>
        </header>
        <div class="history-preview-body" :aria-busy="loading">
          <span v-if="loading" class="history-preview-status" role="status"><LoaderCircle :size="20" class="history-spinner" />{{ t('正在加载…') }}</span>
          <div v-else-if="error" class="history-preview-status" role="alert"><span>{{ te(error) }}</span><Button variant="outline" @click="load">{{ t('重试') }}</Button></div>
          <img v-else-if="image" :key="item?.id" ref="previewImage" :src="image" :alt="t('历史图片原图')" @error="imageFailed" />
          <div v-else-if="color" class="history-preview-color">
            <div class="history-color-checkerboard" role="img" :aria-label="t('颜色预览：{0}', { 0: text.trim() })"><div class="history-color-swatch" :style="{ backgroundColor: color }" /></div>
            <input class="history-color-value" readonly :aria-label="t('颜色值')" :value="text" spellcheck="false" />
          </div>
          <div v-else-if="link" class="history-preview-link">
            <a :href="link" target="_blank" rel="noopener noreferrer" :aria-label="t('在浏览器中打开链接')" :aria-disabled="opening" @click.prevent="openTarget('link')"><ExternalLink :size="16" aria-hidden="true" /><span>{{ text.trim() }}</span></a>
            <span v-if="opening" role="status">{{ t('正在打开…') }}</span>
            <p v-if="openError" role="alert">{{ te(openError) }}</p>
          </div>
          <div v-else-if="path" class="history-preview-path">
            <button type="button" :disabled="opening" :aria-label="t('在资源管理器中打开路径')" :title="t('在资源管理器中打开路径')" @click="openTarget('path')"><FolderOpen :size="20" aria-hidden="true" /><span>{{ path }}</span></button>
            <span v-if="opening" role="status">{{ t('正在打开…') }}</span>
            <p v-if="openError" role="alert">{{ te(openError) }}</p>
          </div>
          <textarea v-else-if="item?.kind === 'text'" class="history-preview-text" readonly :aria-label="t('历史完整文字')" :value="text" spellcheck="false" />
        </div>
      </section>
  </DetachedWindow>
</template>

<style>
.history-preview-dialog { display:flex; flex-direction:column; width:100%; height:100%; overflow:hidden; background:#fff; color:#24292f; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.history-preview-header { display:flex; flex-shrink:0; flex-wrap:wrap; align-items:center; gap:8px 12px; padding:8px 12px; border-bottom:1px solid #e1e4e8; }
.history-preview-header h2 { margin:0; font-size:14px; font-weight:600; }
.history-preview-header p { margin:0; color:#57606a; font-size:12px; }
.history-preview-header button { margin-left:auto; min-width:32px; min-height:32px; padding:4px; }
.history-preview-body { display:flex; align-items:center; justify-content:center; flex:1; min-height:0; padding:16px; overflow:auto; background:#f6f7f8; }
.history-preview-body img { display:block; max-width:100%; max-height:100%; object-fit:contain; }
.history-preview-text { align-self:stretch; flex:1; min-width:0; width:100%; resize:none; padding:16px; border:1px solid #e1e4e8; border-radius:6px; background:#fff; color:#24292f; font:14px/1.7 'Segoe UI','Microsoft YaHei',sans-serif; white-space:pre-wrap; overflow-wrap:anywhere; }
.history-preview-text:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.history-preview-color { display:flex; flex-direction:column; gap:8px; width:100%; max-width:240px; }
.history-color-checkerboard { position:relative; height:120px; overflow:hidden; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; background:repeating-conic-gradient(#ddd 0% 25%, #fff 0% 50%) 0 0 / 20px 20px; }
.history-color-swatch { position:absolute; inset:0; }
.history-color-value { width:100%; min-width:0; box-sizing:border-box; padding:8px 12px; border:1px solid var(--desk-border, #e1e4e8); border-radius:4px; background:var(--desk-bg, #fff); color:inherit; font:14px/1.7 Consolas,monospace; }
.history-color-value:focus-visible { outline:2px solid #2563eb; outline-offset:-2px; }
.history-preview-link { align-self:stretch; flex:1; min-width:0; padding:16px; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; background:var(--desk-bg, #fff); line-height:1.7; user-select:text; }
.history-preview-link a { display:inline-flex; align-items:flex-start; gap:8px; max-width:100%; color:var(--desk-accent, #2563eb); text-decoration:underline; overflow-wrap:anywhere; }
.history-preview-link a svg { flex-shrink:0; margin-top:4px; }
.history-preview-link a:focus-visible { outline:2px solid #2563eb; outline-offset:3px; }
.history-preview-link a[aria-disabled=true] { opacity:.6; cursor:wait; }
.history-preview-link p[role=alert] { color:#b42318; }
.history-preview-path { display:flex; flex-direction:column; align-items:center; gap:8px; width:100%; max-width:640px; min-width:0; line-height:1.7; }
.history-preview-path button { display:flex; align-items:flex-start; gap:10px; max-width:100%; padding:12px 16px; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; background:var(--desk-bg, #fff); color:var(--desk-accent, #2563eb); text-align:left; overflow-wrap:anywhere; }
.history-preview-path button svg { flex-shrink:0; margin-top:2px; }
.history-preview-path button span { min-width:0; }
.history-preview-path button:hover { text-decoration:underline; }
.history-preview-path button:focus-visible { outline:2px solid #2563eb; outline-offset:3px; }
.history-preview-path button:disabled { opacity:.6; cursor:wait; }
.history-preview-path p[role=alert] { max-width:100%; margin:0; color:#b42318; overflow-wrap:anywhere; }
.history-preview-status { display:flex; align-items:center; justify-content:center; flex-wrap:wrap; gap:12px; min-width:0; max-width:100%; overflow-wrap:anywhere; }
.history-spinner { animation:history-spin 1s linear infinite; }
@keyframes history-spin { to { transform:rotate(360deg); } }
@media (prefers-reduced-motion:reduce) { .history-spinner { animation:none; } }
[data-theme=dark] .history-preview-dialog { background:#20252c; color:#e6edf3; border-color:#39424e; }
[data-theme=dark] .history-preview-header { border-color:#39424e; }
[data-theme=dark] .history-preview-header p { color:#a8b3c2; }
[data-theme=dark] .history-preview-body { background:#181b20; }
[data-theme=dark] .history-preview-text { background:#20252c; color:#e6edf3; border-color:#39424e; }
</style>
