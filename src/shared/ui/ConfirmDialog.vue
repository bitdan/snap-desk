<script setup lang="ts">
import DetachedWindow from './DetachedWindow.vue'
import Button from './button/Button.vue'
import { t } from '@/shared/i18n/i18n'
import { handleEscape } from './escape'
import { useEscapeLayer } from './use-escape-layer'

/** 操作确认统一使用独立窗口；提交成功由调用方关闭，失败保留内容供重试。 */
const props = defineProps<{ open: boolean; title: string; description: string; confirmLabel: string; busy?: boolean; destructive?: boolean; error?: string }>()
const emit = defineEmits<{ 'update:open': [open: boolean]; confirm: [] }>()
function close() { if (!props.busy) emit('update:open', false) }
function key(event: KeyboardEvent) { handleEscape(event, close) }
useEscapeLayer(() => props.open, close)
</script>

<template>
  <DetachedWindow :open="open" :title="title" :width="440" :height="300" :busy="busy" @close="close">
      <section class="confirm-dialog" role="alertdialog" :aria-label="title" @keydown.stop="key">
        <h2 class="confirm-dialog-title">{{ title }}</h2>
        <p class="confirm-dialog-description">{{ description }}</p>
        <div v-if="$slots.default" class="confirm-dialog-options"><slot /></div>
        <p v-if="error" class="confirm-dialog-error" role="alert">{{ error }}</p>
        <div class="confirm-dialog-actions">
          <Button variant="outline" :disabled="busy" @click="close">{{ t('取消') }}</Button>
          <Button :variant="destructive ? 'destructive' : 'default'" :disabled="busy" @click="!busy && emit('confirm')">{{ confirmLabel }}</Button>
        </div>
      </section>
  </DetachedWindow>
</template>

<style>
.confirm-dialog { width:100%; min-height:100%; overflow-y:auto; padding:20px; box-sizing:border-box; background:#fff; color:#24292f; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.confirm-dialog-title { margin:0; font-size:16px; font-weight:600; }
.confirm-dialog-description { margin:12px 0 0; color:#57606a; line-height:1.6; white-space:pre-wrap; overflow-wrap:anywhere; }
.confirm-dialog-options { margin-top:12px; }
.confirm-dialog-error { margin-top:12px; color:#b42318; overflow-wrap:anywhere; }
.confirm-dialog-actions { display:flex; flex-wrap:wrap; justify-content:flex-end; gap:8px; margin-top:20px; }
[data-theme=dark] .confirm-dialog { background:#20252c; color:#e6edf3; border-color:#39424e; color-scheme:dark; }
[data-theme=dark] .confirm-dialog-description { color:#a8b3c2; }
[data-theme=dark] .confirm-dialog-error { color:#ff9c95; }
</style>
