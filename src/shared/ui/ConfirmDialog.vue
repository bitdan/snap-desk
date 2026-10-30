<script setup lang="ts">
import { AlertDialogRoot, AlertDialogPortal, AlertDialogOverlay, AlertDialogContent, AlertDialogTitle, AlertDialogDescription, AlertDialogCancel } from 'reka-ui'
import Button from './button/Button.vue'
import { t } from '@/shared/i18n/i18n'
import { handleEscape } from './escape'
import { useEscapeLayer } from './use-escape-layer'

/** 操作确认统一使用模态层管理焦点；提交成功由调用方关闭，失败保留内容供重试。 */
const props = defineProps<{ open: boolean; title: string; description: string; confirmLabel: string; busy?: boolean; destructive?: boolean; error?: string }>()
const emit = defineEmits<{ 'update:open': [open: boolean]; confirm: [] }>()
function close() { if (!props.busy) emit('update:open', false) }
function updateOpen(open: boolean) { if (!open) close() }
function key(event: KeyboardEvent) { handleEscape(event, close) }
function guardDismiss(event: Event) { if (props.busy) event.preventDefault() }
useEscapeLayer(() => props.open, close)
</script>

<template>
  <AlertDialogRoot v-if="open" :open="open" @update:open="updateOpen">
    <AlertDialogPortal>
      <AlertDialogOverlay class="confirm-dialog-overlay" />
      <AlertDialogContent class="confirm-dialog" role="alertdialog" @keydown.stop="key" @escape-key-down="guardDismiss">
        <AlertDialogTitle class="confirm-dialog-title">{{ title }}</AlertDialogTitle>
        <AlertDialogDescription class="confirm-dialog-description">{{ description }}</AlertDialogDescription>
        <div v-if="$slots.default" class="confirm-dialog-options"><slot /></div>
        <p v-if="error" class="confirm-dialog-error" role="alert">{{ error }}</p>
        <div class="confirm-dialog-actions">
          <AlertDialogCancel as-child><Button variant="outline" :disabled="busy">{{ t('取消') }}</Button></AlertDialogCancel>
          <Button :variant="destructive ? 'destructive' : 'default'" :disabled="busy" @click="!busy && emit('confirm')">{{ confirmLabel }}</Button>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>

<style>
.confirm-dialog-overlay { position:fixed; inset:0; z-index:200; background:#10182855; }
.confirm-dialog { position:fixed; z-index:201; top:50%; left:50%; transform:translate(-50%,-50%); width:min(400px, calc(100vw - 32px)); max-height:calc(100vh - 32px); overflow-y:auto; padding:20px; border:1px solid #d8dee4; border-radius:8px; background:#fff; color:#24292f; box-shadow:0 12px 40px #0003; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.confirm-dialog-title { margin:0; font-size:16px; font-weight:600; }
.confirm-dialog-description { margin:12px 0 0; color:#57606a; line-height:1.6; white-space:pre-wrap; overflow-wrap:anywhere; }
.confirm-dialog-options { margin-top:12px; }
.confirm-dialog-error { margin-top:12px; color:#b42318; overflow-wrap:anywhere; }
.confirm-dialog-actions { display:flex; flex-wrap:wrap; justify-content:flex-end; gap:8px; margin-top:20px; }
[data-theme=dark] .confirm-dialog { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .confirm-dialog-description { color:#a8b3c2; }
[data-theme=dark] .confirm-dialog-error { color:#ff9c95; }
</style>
