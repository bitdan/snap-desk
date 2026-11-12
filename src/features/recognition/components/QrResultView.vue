<script setup lang="ts">
import { t, te } from '@/shared/i18n/i18n'
import { Copy, ExternalLink, Link2, LoaderCircle, QrCode, RotateCcw, Type } from 'lucide-vue-next'
import type { QrResult } from '@/features/recognition/types'

/**
 * 截图与贴图共用的二维码结果页：铺满独立窗口，不再套一层卡片。
 * 只负责呈现；复制、打开、重试由宿主执行，宿主决定打开链接前是否确认。
 */
defineProps<{ result: QrResult | null; loading?: boolean; busy?: boolean; error?: string; message?: string; openLabel?: string; retry?: boolean }>()
const emit = defineEmits<{ copy: [index: number]; open: [index: number]; retry: []; close: [] }>()
</script>

<template>
  <div class="qr-view">
    <div class="qr-body">
      <div v-if="loading && !result" class="qr-state" role="status">
        <LoaderCircle :size="24" class="qr-spin" aria-hidden="true" />
        <p>{{ t('正在识别二维码…') }}</p>
      </div>
      <div v-else-if="!result || !result.items.length" class="qr-state">
        <span class="qr-state-icon" aria-hidden="true"><QrCode :size="26" /></span>
        <strong>{{ t('未发现二维码') }}</strong>
        <p v-if="error && !result" role="alert" class="qr-error">{{ te(error) }}</p>
        <p v-else>{{ t('请完整框选二维码并保留周围白边，然后重试。') }}</p>
      </div>
      <template v-else>
        <p class="qr-count">{{ t('识别到 {0} 个二维码', { 0: result.items.length }) }}</p>
        <article v-for="(item, index) in result.items" :key="index" class="qr-item">
          <span class="qr-kind"><component :is="item.url ? Link2 : Type" :size="13" aria-hidden="true" />{{ item.url ? t('链接') : t('文本') }}</span>
          <p class="qr-text" dir="auto">{{ item.text }}</p>
          <div class="qr-actions">
            <button type="button" class="qr-button" :disabled="busy" @click="emit('copy', index)"><Copy :size="14" aria-hidden="true" />{{ t('复制内容') }}</button>
            <button v-if="item.url" type="button" class="qr-button qr-primary" :disabled="busy" @click="emit('open', index)"><ExternalLink :size="14" aria-hidden="true" />{{ t(openLabel ?? '打开') }}</button>
          </div>
        </article>
        <p v-if="error" role="alert" class="qr-error">{{ te(error) }}</p>
      </template>
    </div>
    <footer class="qr-footer">
      <span v-if="message" class="qr-message" role="status">{{ t(message) }}</span>
      <button v-if="retry" type="button" class="qr-button" :disabled="busy" @click="emit('retry')"><RotateCcw :size="14" aria-hidden="true" />{{ t('重新识别') }}</button>
      <button type="button" class="qr-button" @click="emit('close')">{{ t('关闭') }}</button>
    </footer>
  </div>
</template>

<style scoped>
.qr-view { --qr-bg: #ffffff; --qr-soft: #f6f7f9; --qr-border: #e3e6ea; --qr-text: #24292f; --qr-muted: #6b7280; --qr-hover: #eef1f4; --qr-danger: #b42318;
  display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; background: var(--qr-bg); color: var(--qr-text); font: 13px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
:global([data-theme=dark]) .qr-view { --qr-bg: #1b1f24; --qr-soft: #232930; --qr-border: #39424e; --qr-text: #e6edf3; --qr-muted: #9aa7b4; --qr-hover: #2b323c; --qr-danger: #ff8a80; }
.qr-body { display: flex; flex: 1; flex-direction: column; gap: 10px; min-height: 0; padding: 16px; overflow-y: auto; }
.qr-state { display: flex; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding: 16px; text-align: center; }
.qr-state strong { font-size: 15px; font-weight: 600; }
.qr-state p { max-width: 300px; margin: 0; color: var(--qr-muted); line-height: 1.6; }
.qr-state-icon { display: inline-flex; align-items: center; justify-content: center; width: 52px; height: 52px; margin-bottom: 4px; border-radius: 14px; background: var(--qr-soft); color: var(--qr-muted); }
.qr-spin { color: var(--qr-muted); animation: qr-spin 1s linear infinite; }
@keyframes qr-spin { to { transform: rotate(360deg); } }
.qr-count { margin: 0; color: var(--qr-muted); font-size: 12px; }
.qr-item { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--qr-border); border-radius: 8px; background: var(--qr-soft); }
.qr-kind { display: inline-flex; align-items: center; gap: 4px; color: var(--qr-muted); font-size: 12px; }
.qr-text { max-height: 7.2em; margin: 0; overflow-y: auto; line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
.qr-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
.qr-footer { display: flex; align-items: center; justify-content: flex-end; gap: 6px; padding: 10px 16px; border-top: 1px solid var(--qr-border); }
.qr-message { margin-right: auto; color: var(--qr-muted); font-size: 12px; }
.qr-error { margin: 0; color: var(--qr-danger); overflow-wrap: anywhere; }
.qr-button { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px; border: 1px solid var(--qr-border); border-radius: 6px; background: var(--qr-bg); color: inherit; font: inherit; cursor: pointer; }
.qr-button:hover:enabled { background: var(--qr-hover); }
.qr-button:disabled { opacity: .45; cursor: default; }
.qr-button:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.qr-primary { border-color: #2563eb; background: #2563eb; color: #fff; }
.qr-primary:hover:enabled { background: #1d4ed8; }
</style>
