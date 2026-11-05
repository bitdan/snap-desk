<script setup lang="ts">
import { Check, Cloud, HardDrive, Server } from 'lucide-vue-next'
import { t } from '@/shared/i18n/i18n'
import type { ConnectionMode } from '../services'
/** 功能设置内的常驻二级导航；只负责选择，具体配置与保存由所属页面处理。 */
export interface ProviderMenuItem { value: string; label: string; description?: string; disabled?: boolean; connection?: ConnectionMode; active?: boolean }
defineProps<{ modelValue: string; options: ProviderMenuItem[]; label: string; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
</script>

<template>
  <nav class="recognition-provider-menu" :aria-label="label">
    <div class="recognition-menu-label">{{ label }}</div>
    <button v-for="item in options" :key="item.value" type="button" :data-provider="item.value" :title="[item.label, item.description, item.connection === 'offline' ? t('离线') : item.connection === 'online' ? t('在线 API') : item.connection ? t('本机 API') : '', item.active ? t('当前使用') : ''].filter(Boolean).join(' · ')" :aria-current="modelValue === item.value ? 'page' : undefined" :disabled="disabled || item.disabled" @click="emit('update:modelValue', item.value)">
      <span class="recognition-menu-name">{{ item.label }}</span>
      <span class="recognition-menu-meta">
        <span v-if="item.connection" class="recognition-connection-badge" :data-connection="item.connection" :title="item.connection === 'offline' ? t('离线') : item.connection === 'online' ? t('在线 API') : t('本机 API')"><component :is="item.connection === 'offline' ? HardDrive : item.connection === 'online' ? Cloud : Server" :size="13" aria-hidden="true" /><span class="sr-only">{{ item.connection === 'offline' ? t('离线') : item.connection === 'online' ? t('在线 API') : t('本机 API') }}</span></span>
        <span class="recognition-active-provider"><span v-if="item.active" data-active-provider :title="t('当前使用')"><Check :size="13" aria-hidden="true" /><span class="sr-only">{{ t('当前使用') }}</span></span></span>
      </span>
    </button>
  </nav>
</template>

<style>
/* 名称、连接类型和当前状态同排；图标保留读屏文字，不重复占用第二行。 */
.recognition-settings-layout { display:grid; grid-template-columns:184px minmax(0,1fr); gap:24px; align-items:start; }
.recognition-settings-sidebar { min-width:0; padding-right:12px; border-right:1px solid var(--desk-border,#e1e4e8); }
.recognition-settings-content { min-width:0; max-width:720px; }
.recognition-provider-menu { display:flex; flex-direction:column; gap:4px; margin-bottom:16px; }
.recognition-menu-label { padding:0 10px 8px; color:var(--desk-muted,#57606a); font-size:12px; }
.recognition-provider-menu button { display:flex; gap:8px; align-items:center; width:100%; min-height:36px; padding:7px 8px; border:1px solid transparent; border-radius:5px; background:transparent; color:var(--desk-text,#24292f); font:inherit; font-size:13px; text-align:left; cursor:pointer; }
.recognition-provider-menu button:hover:enabled { background:var(--desk-soft,#f6f7f8); }
.recognition-provider-menu button[aria-current=page] { background:var(--desk-selected,#eef4ff); border-color:color-mix(in srgb,var(--desk-accent,#2563eb) 20%,transparent); color:var(--desk-accent,#2563eb); }
.recognition-provider-menu button:focus-visible { outline:2px solid var(--desk-accent,#2563eb); outline-offset:2px; }
.recognition-provider-menu button:disabled { opacity:.5; cursor:default; }
.recognition-menu-name { flex:1; min-width:0; line-height:18px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.recognition-menu-meta { display:flex; align-items:center; gap:6px; flex:none; white-space:nowrap; }
.recognition-current-provider { margin:0 0 16px; padding:0 0 12px; border-bottom:1px solid var(--desk-border,#e1e4e8); color:var(--desk-text,#24292f); font-size:13px; line-height:1.6; overflow-wrap:anywhere; }
.recognition-active-provider { width:13px; min-height:13px; color:var(--ui-success-fg,#24834b); }
.recognition-connection-badge { display:inline-flex; align-items:center; gap:4px; flex:none; color:var(--desk-muted,#57606a); font-size:11px; font-weight:400; line-height:16px; }
.recognition-connection-badge[data-connection=online] { color:var(--desk-accent,#2563eb); }
@media (max-width:700px) { .recognition-settings-layout { grid-template-columns:160px minmax(0,1fr); gap:16px; } .recognition-settings-sidebar { padding-right:8px; } }
@media (max-width:480px) { .recognition-settings-layout { grid-template-columns:minmax(0,1fr); } .recognition-settings-sidebar { border-right:0; border-bottom:1px solid var(--desk-border,#e1e4e8); padding:0 0 8px; } .recognition-provider-menu { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); margin-bottom:8px; } .recognition-menu-label { grid-column:1/-1; } }
</style>
